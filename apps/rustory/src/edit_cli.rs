//! The agent-facing editing interface.
//!
//! Everything here is designed around one question: **can a program drive this
//! without a human reading the output?**
//!
//! Three consequences, all deliberate:
//!
//! * **Discoverable.** `rustory ops` prints the operation vocabulary, so an agent
//!   learns what it can do from the program rather than from a prompt someone wrote.
//! * **Structured.** Every command has a `--json` form whose shape does not change
//!   between success and failure. An agent parses one schema, not two.
//! * **Atomic.** `edit` either applies every operation and writes the file, or
//!   changes nothing and reports why. There is no partial write to recover from.

use std::path::Path;

use rustory_edit::{BatchError, EditOp, Editor};
use serde::Serialize;

/// What every command returns, so a caller parses one shape.
#[derive(Debug, Serialize)]
pub struct Report {
    /// Whether the command achieved its purpose.
    pub ok: bool,
    /// One line a human can read in a terminal.
    pub message: String,
    /// Whether the file on disk changed.
    pub changed: bool,
    /// Problems found, structured so a caller can act on them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
}

impl Report {
    fn ok(message: impl Into<String>, changed: bool) -> Self {
        Self {
            ok: true,
            message: message.into(),
            changed,
            problems: Vec::new(),
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            changed: false,
            problems: Vec::new(),
        }
    }
}

/// `rustory ops` — the operation vocabulary.
pub fn ops(as_json: bool) -> Report {
    let catalogue = rustory_edit::catalogue();

    if as_json {
        // Serialised from the catalogue rather than rebuilt field by field: a second
        // hand-maintained copy of this shape is exactly how the example would go missing
        // from the CLI while existing in the library.
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "count": catalogue.len(),
                "operations": catalogue,
            })
        );
        return Report::ok(format!("{} operations", catalogue.len()), false);
    }

    println!("{} operations:", catalogue.len());
    for info in &catalogue {
        let marker = if info.changes_structure { "*" } else { " " };
        println!("{marker} {:<18} {}", info.name, info.summary);
        // The example goes right under the description: an agent reading `ops` is one
        // paste away from a correct operation instead of guessing field names.
        println!("  {}", info.example);
    }
    println!("\n* marks operations that add or remove nodes, beats or options.");
    println!("Each example above parses as written: change the values, keep the shape.");
    Report::ok(format!("{} operations", catalogue.len()), false)
}

/// `rustory inspect <story.ron>` — the document as data.
pub fn inspect(path: &Path, as_json: bool) -> Report {
    let editor = match Editor::open(path) {
        Ok(editor) => editor,
        Err(err) => {
            if as_json {
                println!(
                    "{}",
                    serde_json::json!({"ok": false, "message": err.to_string()})
                );
            }
            return Report::failed(err.to_string());
        }
    };

    let story = editor.story();
    let problems = editor.problems();

    if as_json {
        // Built by hand rather than by deriving Serialize on Story: the point is a
        // shape that is convenient to consume, not one that mirrors the in-memory
        // types. Nodes carry their own id so a consumer never has to join keys.
        let nodes: Vec<serde_json::Value> = story
            .nodes
            .iter()
            .map(|(id, node)| {
                serde_json::json!({
                    "id": id.as_str(),
                    "beats": node.beats,
                    "beat_count": node.beats.len(),
                    "exit": node.exit,
                })
            })
            .collect();

        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "file": editor.path().display().to_string(),
                "title": story.meta.title,
                "author": story.meta.author,
                "start": story.start.as_str(),
                "node_count": story.nodes.len(),
                "beat_count": story.beat_count(),
                "nodes": nodes,
                "problems": problems,
            })
        );
        return Report {
            ok: true,
            message: format!("{} nodes, {} beats", story.nodes.len(), story.beat_count()),
            changed: false,
            problems,
        };
    }

    println!("{}", editor.path().display());
    if let Some(title) = &story.meta.title {
        println!("title: {title}");
    }
    println!("start: {}", story.start);
    println!("{} nodes, {} beats", story.nodes.len(), story.beat_count());

    for (id, node) in &story.nodes {
        let marker = if id == &story.start { ">" } else { " " };
        println!("{marker} {id}  ({} beats)", node.beats.len());
        for (index, beat) in node.beats.iter().enumerate() {
            println!("    {index}: {}", describe_beat(beat));
        }
        println!("    exit: {}", describe_exit(&node.exit));
    }

    if !problems.is_empty() {
        println!("\n{} problem(s):", problems.len());
        for problem in &problems {
            println!("  - {problem}");
        }
    }

    Report {
        ok: true,
        message: format!("{} nodes, {} beats", story.nodes.len(), story.beat_count()),
        changed: false,
        problems,
    }
}

/// `rustory edit <story.ron> --op <json>` — apply operations and write.
///
/// Writing is the default because that is what a caller asking to edit wants. A
/// caller who wants to check first passes `--dry-run`.
pub fn edit(path: &Path, ops: &[EditOp], write: bool, as_json: bool) -> Report {
    let mut editor = match Editor::open(path) {
        Ok(editor) => editor,
        Err(err) => {
            emit_failure(as_json, &err.to_string());
            return Report::failed(err.to_string());
        }
    };

    match editor.apply_all(ops) {
        Ok(count) => {
            if write && let Err(err) = editor.save() {
                emit_failure(as_json, &err.to_string());
                return Report::failed(err.to_string());
            }

            let problems = editor.problems();
            let message = if write {
                format!("applied {count} operation(s) and saved {}", path.display())
            } else {
                format!("applied {count} operation(s) in memory (dry run, file untouched)")
            };

            if as_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "applied": count,
                        "written": write,
                        "file": path.display().to_string(),
                        "problems": problems,
                    })
                );
            }
            Report {
                ok: true,
                message,
                changed: write,
                problems,
            }
        }
        Err(BatchError { index, source }) => {
            let message = format!("operation {index} was refused: {source}");
            if as_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": false,
                        "failed_at": index,
                        "message": message,
                        "written": false,
                    })
                );
            }
            Report::failed(message)
        }
    }
}

fn emit_failure(as_json: bool, message: &str) {
    if as_json {
        println!("{}", serde_json::json!({"ok": false, "message": message}));
    }
    eprintln!("rustory: {message}");
}

/// Parse one operation from JSON.
pub fn parse_op(text: &str) -> Result<EditOp, String> {
    serde_json::from_str(text).map_err(|err| {
        format!(
            "cannot read the operation as JSON: {err}\n\
             Run `rustory ops --json` for the vocabulary."
        )
    })
}

/// Parse a JSON array of operations.
pub fn parse_ops(text: &str) -> Result<Vec<EditOp>, String> {
    serde_json::from_str(text).map_err(|err| {
        format!(
            "cannot read the operation batch as JSON: {err}\n\
             Expected an array of operations; see `rustory ops --json`."
        )
    })
}

fn describe_beat(beat: &rustory_core::Beat) -> String {
    use rustory_core::Beat;
    match beat {
        Beat::Say { speaker, text } => match speaker {
            Some(who) => format!("{who}: {text}"),
            None => format!("(narration) {text}"),
        },
        Beat::ShowActor { actor, pose, slot } => {
            format!("show {actor} as `{pose}` at {slot:?}")
        }
        Beat::HideActor { actor } => format!("hide {actor}"),
        Beat::SetBackground { image } => match image {
            Some(image) => format!("background {image}"),
            None => "clear background".to_owned(),
        },
        Beat::Music { track, fade_ms } => match (track, fade_ms.unwrap_or(0)) {
            (Some(track), 0) => format!("music {track}"),
            (Some(track), fade) => format!("music {track} over {fade}ms"),
            (None, 0) => "stop music".to_owned(),
            (None, fade) => format!("stop music over {fade}ms"),
        },
        Beat::Sound { clip, volume } => match volume {
            Some(volume) => format!("sound {clip} at {volume:.2}"),
            None => format!("sound {clip}"),
        },
    }
}

fn describe_exit(exit: &rustory_core::Exit) -> String {
    use rustory_core::Exit;
    match exit {
        Exit::End { ending } => match ending {
            Some(ending) => format!("end ({ending})"),
            None => "end".to_owned(),
        },
        Exit::Goto { to } => format!("goto {to}"),
        Exit::Choose { choices, .. } => format!(
            "choose from {} option(s): {}",
            choices.len(),
            choices
                .iter()
                .map(|choice| format!("`{}` -> {}", choice.label, choice.to))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Exit::Gameplay { spec } => format!(
            "gameplay {} -> {}",
            spec.system,
            spec.outcomes
                .iter()
                .map(|(outcome, to)| format!("{outcome}->{to}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
