//! `rustory` — run a RuStory script, or inspect one.
//!
//! The CLI stays small on purpose: the engine is the deliverable, and a CLI that
//! grows into a build system would distract from it.

#![forbid(unsafe_code)]

mod assets;
mod driver;
mod edit_cli;
mod play;
mod scene;

use std::path::{Path, PathBuf};

use rustory_core::{Narrative, NodeId};
use rustory_render::{Renderer, Rgba8Image};
use rustory_script::ScriptError;

use crate::scene::{HEIGHT, WIDTH};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("rustory: {message}");
            1
        }
    };
    std::process::exit(code);
}

const USAGE: &str = "\
rustory — Rust-native visual novel engine

USAGE:
    rustory probe                       Check the GPU and font stack
    rustory check <story.ron>           Parse and validate a script
    rustory transcript <story.ron>      Play it headlessly and print the lines
    rustory gen-assets [dir]            Write placeholder art (default: assets)
    rustory shot <story.ron> [options]  Render one frame to a PNG
    rustory play <story.ron> [options]  Play it in a window
    rustory save <story.ron> [options]  Advance, then write a save file
    rustory resume <story.ron> <save>   Load a save and report where it resumes
    rustory editor [story.ron]          Open the visual editor

EDITING (the agent interface):
    rustory ops [--json]                List every edit operation
    rustory inspect <story.ron> [--json]  Print the story as data
    rustory edit <story.ron> --op <json>  Apply one operation and save
    rustory edit <story.ron> --ops <json-file>  Apply a batch
        --dry-run      Validate without writing the file
        --json         Machine-readable output

OPTIONS for shot / play / save / resume:
    --assets <dir>     Where sprite and background PNGs live (default: assets)
    --out <file>       Output path for shot (.png) or save (.ron)
    --advance <n>      Clicks to simulate before acting (default: 0)
    --at <node>[:<beat>]  Stop at a named position instead of counting clicks
    --finish           resume: keep playing to an ending instead of reporting
    --font <family>    Typeface for all text; load files from <assets>/fonts first
                       (no --font means the platform's default sans-serif)
";

fn run(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first() else {
        print!("{USAGE}");
        return Ok(());
    };

    match command.as_str() {
        "probe" => probe(),
        "check" => {
            let path = args.get(1).ok_or("check needs a story path")?;
            check(Path::new(path))
        }
        "transcript" => {
            let path = args.get(1).ok_or("transcript needs a story path")?;
            transcript(Path::new(path))
        }
        "gen-assets" => {
            let dir = args
                .get(1)
                .map_or_else(|| PathBuf::from("assets"), PathBuf::from);
            let written = assets::generate(&dir).map_err(|err| err.to_string())?;
            for path in written {
                println!("wrote {}", path.display());
            }
            Ok(())
        }
        "shot" => shot(&args[1..]),
        "play" => play::run(&args[1..]),
        "save" => save_cmd(&args[1..]),
        "resume" => resume_cmd(&args[1..]),
        "ops" => {
            let as_json = args.iter().any(|arg| arg == "--json");
            finish_quiet(edit_cli::ops(as_json), as_json)
        }
        "inspect" => {
            let path = args.get(1).ok_or("inspect needs a story path")?;
            let as_json = args.iter().any(|arg| arg == "--json");
            finish_quiet(edit_cli::inspect(Path::new(path), as_json), as_json)
        }
        "edit" => edit_cmd(&args[1..]),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn probe() -> Result<(), String> {
    println!("rustory {}", env!("CARGO_PKG_VERSION"));
    match rustory_render::probe_gpu() {
        Ok(adapter) => println!("gpu:  {adapter}"),
        Err(err) => println!("gpu:  UNAVAILABLE — {err}"),
    }
    println!("text: {}", rustory_render::probe_text());

    // Report project fonts as well as system ones: "the font did not apply" is
    // otherwise a guess about whether the file loaded or the name was wrong.
    if let Ok(mut renderer) = Renderer::new_headless(8, 8) {
        let loaded = driver::load_fonts(&mut renderer, Path::new("assets"));
        if loaded.is_empty() {
            println!("project fonts: none in assets/fonts (using platform defaults)");
        } else {
            println!("project fonts: {}", loaded.join(", "));
        }
    }
    Ok(())
}

fn load(path: &Path) -> Result<rustory_core::Story, String> {
    rustory_script::load_story(path).map_err(|err| match err {
        ScriptError::Io { .. } => format!("cannot read {}", path.display()),
        other => other.to_string(),
    })
}

fn check(path: &Path) -> Result<(), String> {
    let story = load(path)?;

    let choices = story
        .nodes
        .values()
        .filter(|node| matches!(node.exit, rustory_core::Exit::Choose { .. }))
        .count();
    let gameplay = story
        .nodes
        .values()
        .filter(|node| matches!(node.exit, rustory_core::Exit::Gameplay { .. }))
        .count();

    println!("ok: {}", path.display());
    println!(
        "{} nodes, {} beats, {} choice points, {} gameplay handovers",
        story.nodes.len(),
        story.beat_count(),
        choices,
        gameplay
    );
    if let Some(title) = &story.meta.title {
        println!("title: {title}");
    }

    // Warn about systems the story invokes but nothing registered: a load-time
    // failure is much cheaper than discovering it three scenes in.
    let registry = driver::default_registry();
    for system in driver::required_systems(&story) {
        if !registry.contains(&rustory_core::SystemId::from(system.as_str())) {
            println!("warning: gameplay system `{system}` is not registered");
        }
    }
    for (system, outcomes) in driver::mapped_outcomes(&story) {
        println!(
            "{system}: handles {}",
            outcomes
                .iter()
                .map(|key| key.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}

fn transcript(path: &Path) -> Result<(), String> {
    let story = load(path)?;
    let lines = driver::transcript(story, 500)?;
    for line in &lines {
        println!("{line}");
    }
    println!("---");
    println!("{} lines", lines.len());
    Ok(())
}

/// Where `--at` points: a node, and optionally a beat inside it.
///
/// `prologue` means "arrive at that node", which is the common case; `prologue:3` also
/// asks for the fourth beat. The beat is a runtime index, not a script index: entering
/// a node runs its leading non-blocking beats (a background, an actor) before the first
/// line appears, so `prologue:0` is usually already behind you.
fn parse_at(value: &str) -> (NodeId, Option<usize>) {
    if let Some((node, beat)) = value.rsplit_once(':')
        && let Ok(beat) = beat.parse::<usize>()
    {
        return (NodeId::from(node), Some(beat));
    }
    (NodeId::from(value), None)
}

/// Advance until the narrative is showing `target`.
///
/// Deliberately not "click n times": a click finishes a revealing line without moving
/// on, and inside a gameplay system a click plays the whole interaction to completion,
/// so click counts do not map onto positions.
fn seek(
    narrative: &mut Narrative,
    registry: &mut rustory_core::GameplayRegistry,
    target: &str,
) -> Result<(), String> {
    let (node, beat) = parse_at(target);
    if !narrative.story().nodes.contains_key(&node) {
        return Err(format!("no node `{node}` in this story"));
    }

    const LIMIT: usize = 1000;
    for _ in 0..LIMIT {
        if narrative.cursor() == &node {
            match beat {
                // Arriving is the whole request when no beat was named.
                None => return Ok(()),
                Some(beat) if narrative.beat_index() == beat => return Ok(()),
                // Forwards only: rewinding would need a second story position, and a
                // tool that silently restarts is worse than one that says no.
                Some(beat) if narrative.beat_index() > beat => {
                    return Err(format!(
                        "`{node}` is already past beat {beat} (showing {}); `--at` moves forward only",
                        narrative.beat_index()
                    ));
                }
                Some(_) => {}
            }
        }
        if matches!(narrative.mode(), rustory_core::Mode::Finished { .. }) {
            return Err(format!("the story ended before reaching `{target}`"));
        }
        driver::click(narrative, registry)?;
    }
    Err(format!(
        "`{target}` was not reached in {LIMIT} interactions"
    ))
}

struct ShotOptions {
    story: PathBuf,
    assets: PathBuf,
    out: PathBuf,
    advance: usize,
    /// Stop here instead of counting clicks: `node` or `node:beat`.
    at: Option<String>,
    font: Option<String>,
}

fn parse_shot_args(args: &[String]) -> Result<ShotOptions, String> {
    let mut story = None;
    let mut options = ShotOptions {
        story: PathBuf::new(),
        assets: PathBuf::from("assets"),
        out: PathBuf::from("shot.png"),
        advance: 0,
        at: None,
        font: None,
    };

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--assets" => {
                index += 1;
                options.assets = PathBuf::from(args.get(index).ok_or("--assets needs a path")?);
            }
            "--out" => {
                index += 1;
                options.out = PathBuf::from(args.get(index).ok_or("--out needs a path")?);
            }
            "--advance" => {
                index += 1;
                let value = args.get(index).ok_or("--advance needs a number")?;
                options.advance = value
                    .parse()
                    .map_err(|_| format!("--advance expects a number, got `{value}`"))?;
            }
            "--at" => {
                index += 1;
                options.at = Some(
                    args.get(index)
                        .ok_or("--at needs a node, optionally `node:beat`")?
                        .clone(),
                );
            }
            "--font" => {
                index += 1;
                options.font = Some(args.get(index).ok_or("--font needs a family name")?.clone());
            }
            other if other.starts_with("--") => return Err(format!("unknown option `{other}`")),
            other => {
                if story.is_some() {
                    return Err(format!("unexpected argument `{other}`"));
                }
                story = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }

    options.story = story.ok_or("shot needs a story path")?;
    Ok(options)
}

fn shot(args: &[String]) -> Result<(), String> {
    let options = parse_shot_args(args)?;
    let story = load(&options.story)?;
    let mut narrative = Narrative::new(story).map_err(|err| err.to_string())?;
    let mut registry = driver::default_registry();

    if let Some(target) = &options.at {
        if options.advance > 0 {
            return Err("--at and --advance both position the frame; use one".into());
        }
        seek(&mut narrative, &mut registry, target)?;
    } else {
        for _ in 0..options.advance {
            if matches!(narrative.mode(), rustory_core::Mode::Finished { .. }) {
                break;
            }
            driver::click(&mut narrative, &mut registry)?;
        }
    }

    // Capture what the player would actually read: finish revealing the current
    // line, otherwise every shot taken mid-reveal shows an empty dialogue box.
    narrative.tick(0, 0);

    // A suspended narrative has not handed control to the system yet, so its
    // prompt is empty. A zero-duration step performs the handover without
    // consuming the clock.
    if matches!(narrative.mode(), rustory_core::Mode::Suspended { .. }) {
        let _ = narrative.step_gameplay(&mut registry, 0, &[]);
    }

    let mut renderer = Renderer::new_headless(WIDTH, HEIGHT).map_err(|err| err.to_string())?;
    let bank = driver::load_textures(&mut renderer, &options.assets);
    let loaded = driver::load_fonts(&mut renderer, &options.assets);
    if let Some(family) = driver::choose_font(&mut renderer, options.font.as_deref(), &loaded)? {
        println!("font: {family}");
    }

    let prompt = narrative.gameplay_prompt(&registry);
    let frame = driver::build_frame(&narrative, &bank, prompt.as_deref(), None);

    let image = renderer
        .render_offscreen(&frame)
        .map_err(|err| err.to_string())?;
    image
        .save_png(&options.out)
        .map_err(|err| err.to_string())?;

    println!(
        "wrote {} ({}x{}, {} primitives, {})",
        options.out.display(),
        image.width(),
        image.height(),
        frame.primitive_count(),
        driver::describe_mode(&narrative)
    );
    Ok(())
}

/// Advance a story, then write a save.
///
/// `--advance` counts interactions the same way `shot` does, which makes it
/// possible to reach an interesting state (mid-line, mid-choice, mid-gameplay)
/// without a window.
fn save_cmd(args: &[String]) -> Result<(), String> {
    let mut story_path = None;
    let mut out = PathBuf::from("save.ron");
    let mut advance = 0usize;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                out = PathBuf::from(args.get(index).ok_or("--out needs a path")?);
            }
            "--advance" => {
                index += 1;
                let value = args.get(index).ok_or("--advance needs a number")?;
                advance = value
                    .parse()
                    .map_err(|_| format!("--advance expects a number, got `{value}`"))?;
            }
            other if other.starts_with("--") => return Err(format!("unknown option `{other}`")),
            other => {
                if story_path.is_some() {
                    return Err(format!("unexpected argument `{other}`"));
                }
                story_path = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }

    let story_path = story_path.ok_or("save needs a story path")?;
    let story = load(&story_path)?;
    let mut narrative = Narrative::new(story).map_err(|err| err.to_string())?;
    let mut registry = driver::default_registry();

    for _ in 0..advance {
        if matches!(narrative.mode(), rustory_core::Mode::Finished { .. }) {
            break;
        }
        driver::click(&mut narrative, &mut registry)?;
    }

    // A script that reaches a gameplay handover leaves the mode suspended but the
    // system un-begun: `begin` runs on the first step. Saving in that window would
    // record "no state" for a system that is about to have some. A zero-length step
    // performs the handover without consuming the clock.
    if matches!(narrative.mode(), rustory_core::Mode::Suspended { .. }) {
        narrative
            .step_gameplay(&mut registry, 0, &[])
            .map_err(|err| err.to_string())?;
    }

    let save = narrative.save(&registry).map_err(|err| err.to_string())?;
    let text = save.to_ron().map_err(|err| err.to_string())?;
    std::fs::write(&out, text).map_err(|err| format!("cannot write {}: {err}", out.display()))?;

    println!(
        "wrote {} at node `{}` ({})",
        out.display(),
        narrative.cursor(),
        driver::describe_mode(&narrative)
    );
    Ok(())
}

/// Load a save and report where it resumes; with `--finish`, keep playing.
fn resume_cmd(args: &[String]) -> Result<(), String> {
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut finish = false;

    for arg in args {
        match arg.as_str() {
            "--finish" => finish = true,
            other if other.starts_with("--") => return Err(format!("unknown option `{other}`")),
            other => positional.push(PathBuf::from(other)),
        }
    }

    let [story_path, save_path] = positional.as_slice() else {
        return Err("resume needs a story path and a save path".into());
    };

    let story = load(story_path)?;
    let text = std::fs::read_to_string(save_path)
        .map_err(|err| format!("cannot read {}: {err}", save_path.display()))?;
    let save = rustory_core::SaveState::from_ron(&text).map_err(|err| err.to_string())?;

    let mut registry = driver::default_registry();
    let mut narrative =
        Narrative::from_save(story, &save, &mut registry).map_err(|err| err.to_string())?;

    println!(
        "resumed at node `{}` ({})",
        narrative.cursor(),
        driver::describe_mode(&narrative)
    );
    if let Some(line) = narrative.line() {
        println!("line: {}", scene::line_summary(&line));
    }

    if finish {
        // Bounded so a story that never ends cannot hang the CLI; the same guard
        // the transcript driver uses.
        for _ in 0..10_000 {
            if matches!(narrative.mode(), rustory_core::Mode::Finished { .. }) {
                break;
            }
            driver::click(&mut narrative, &mut registry)?;
        }
        println!("then: {}", driver::describe_mode(&narrative));
    }

    Ok(())
}

#[allow(dead_code)]
fn unused(_: Rgba8Image) {}

/// Turn a report into an exit code.
///
/// A single place so every editing command agrees on what failure looks like: an
/// agent checking `$?` and an agent reading `"ok": false` must never disagree.
/// Print the human summary unless JSON was asked for.
///
/// In JSON mode stdout must contain the document and nothing else: a parser that finds
/// "17 operations" after the closing brace is entitled to call the output broken, and
/// it would be right. The summary still goes to stderr so a person watching the terminal
/// is not left with silence.
fn finish_quiet(report: edit_cli::Report, quiet: bool) -> Result<(), String> {
    if report.ok {
        if quiet {
            if !report.problems.is_empty() {
                eprintln!("{}", report.problems.join("; "));
            }
        } else if report.problems.is_empty() {
            println!("{}", report.message);
        } else {
            println!("{}", report.problems.join("; "));
        }
        Ok(())
    } else {
        Err(report.message)
    }
}

/// `rustory edit <story.ron> (--op JSON | --ops FILE) [--dry-run] [--json]`
fn edit_cmd(args: &[String]) -> Result<(), String> {
    let mut story_path: Option<PathBuf> = None;
    let mut single: Option<String> = None;
    let mut batch: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut as_json = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--op" => {
                index += 1;
                single = Some(args.get(index).ok_or("--op needs JSON")?.clone());
            }
            "--ops" => {
                index += 1;
                batch = Some(PathBuf::from(args.get(index).ok_or("--ops needs a path")?));
            }
            "--dry-run" => dry_run = true,
            "--json" => as_json = true,
            other if other.starts_with("--") => return Err(format!("unknown option `{other}`")),
            other => {
                if story_path.is_some() {
                    return Err(format!("unexpected argument `{other}`"));
                }
                story_path = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }

    let story_path = story_path.ok_or("edit needs a story path")?;

    // Accepting both would make the order of application a guess.
    let ops = match (single, batch) {
        (Some(_), Some(_)) => return Err("use either --op or --ops, not both".into()),
        (None, None) => return Err("edit needs --op <json> or --ops <file>".into()),
        (Some(json), None) => vec![edit_cli::parse_op(&json)?],
        (None, Some(path)) => {
            let text = std::fs::read_to_string(&path)
                .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
            edit_cli::parse_ops(&text)?
        }
    };

    finish_quiet(
        edit_cli::edit(&story_path, &ops, !dry_run, as_json),
        as_json,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_node_name_means_arrive_at_the_node() {
        assert_eq!(parse_at("prologue"), (NodeId::from("prologue"), None));
    }

    #[test]
    fn a_colon_names_a_beat_inside_the_node() {
        assert_eq!(parse_at("prologue:3"), (NodeId::from("prologue"), Some(3)));
    }

    #[test]
    fn a_colon_without_a_number_stays_part_of_the_node_name() {
        // Node ids are chosen by authors, so a colon that is not a beat must not be
        // quietly eaten: `act:one` is one node, not node `act` beat `one`.
        assert_eq!(parse_at("act:one"), (NodeId::from("act:one"), None));
    }
}
