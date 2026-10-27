//! The visual editor.
//!
//! # What this program is
//!
//! A front-end. Every change it makes goes through [`rustory_vn_edit::EditOp`], the same
//! type the command line produces, so a person clicking a button and an agent issuing
//! JSON are editing through one code path and cannot drift apart.
//!
//! That constraint shapes the whole design: the editor owns *selection and layout*,
//! never game state. If a feature is not expressible as an `EditOp`, it does not
//! belong in the editor — it belongs in the operations, where the CLI gets it too.
//!
//! # Preview
//!
//! The preview panel is the real engine drawing the real story into a texture. It is
//! not a mock-up: `rustory_vn_render::Renderer` renders the same `Frame` that
//! `rustory play` would show. A preview that approximated the output would be worse
//! than none, because it would be trusted.
//!
//! # Verifying it
//!
//! `rustory-editor --check <story.ron>` renders one preview frame without a window and
//! reports whether the result looks like a scene. Without that, the only way to know
//! the preview works is for a human to look at it.

#![forbid(unsafe_code)]

mod app;
mod check;
mod dialogs;
mod outcomes;
mod panels;
mod preview;
mod recent;
mod tab;

use std::path::PathBuf;

const USAGE: &str = "\
rustory-vn-editor — visual editor for RuStory

USAGE:
    rustory-vn-editor [story.ron]            Open the editor
    rustory-editor --check <story.ron>    Render one preview frame and report
        --step <n>      interactions to advance before rendering (default: 0)
        --out <file>    write the frame as a PNG
    rustory-editor --recent               List recent stories, newest first
        --json          print as JSON instead of one path per line
";

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return Ok(());
    }

    // `--check` is a terminal mode: it never opens a window, so it works over SSH and
    // in CI where there is no display.
    if args.iter().any(|arg| arg == "--check") {
        std::process::exit(run_check(&args));
    }

    // `--recent` is terminal too. An agent asking "what has the user been editing?"
    // should not need a display server to find out.
    if args.iter().any(|arg| arg == "--recent") {
        std::process::exit(run_recent(&args));
    }

    let story = first_positional(&args);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1000.0, 640.0])
            .with_title("RuStory Editor"),
        // The preview needs a real GPU device; the default renderer for this build is
        // wgpu, which is the same backend the engine uses.
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "RuStory Editor",
        options,
        Box::new(move |cc| Ok(Box::new(app::EditorApp::new(cc, story.clone())))),
    )
}

fn first_positional(args: &[String]) -> Option<String> {
    args.iter()
        .find(|arg| !arg.starts_with("--"))
        .map(|arg| arg.to_string())
}

/// Handle `--recent`, returning an exit code.
///
/// Deliberately reads without pruning: a terminal command that silently rewrites the
/// user's configuration file is a surprise, and the list entry that cannot open will be
/// dropped by the editor the next time it runs.
fn run_recent(args: &[String]) -> i32 {
    let json = args.iter().any(|arg| arg == "--json");
    let recent = recent::RecentFiles::load();
    print!("{}", recent::render(recent.paths(), json));
    if !json && recent.is_empty() {
        eprintln!("rustory-vn-editor: no recent stories");
    }
    0
}

/// Handle `--check`, returning an exit code.
fn run_check(args: &[String]) -> i32 {
    let mut story_path: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut step = 0usize;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--check" => {}
            "--step" => {
                index += 1;
                match args.get(index).and_then(|value| value.parse().ok()) {
                    Some(value) => step = value,
                    None => {
                        eprintln!("rustory-vn-editor: --step needs a number");
                        return 2;
                    }
                }
            }
            "--out" => {
                index += 1;
                match args.get(index) {
                    Some(value) => out = Some(PathBuf::from(value)),
                    None => {
                        eprintln!("rustory-vn-editor: --out needs a path");
                        return 2;
                    }
                }
            }
            other if other.starts_with("--") => {
                eprintln!("rustory-vn-editor: unknown option `{other}`");
                return 2;
            }
            other => {
                if story_path.is_some() {
                    eprintln!("rustory-vn-editor: unexpected argument `{other}`");
                    return 2;
                }
                story_path = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }

    let Some(story_path) = story_path else {
        eprintln!("rustory-vn-editor: --check needs a story path");
        return 2;
    };

    let story = match rustory_vn_script::load_story(&story_path) {
        Ok(story) => story,
        Err(err) => {
            eprintln!("rustory-vn-editor: {err}");
            return 1;
        }
    };

    match check::run(&story, step, out.as_deref()) {
        Ok(report) => {
            println!(
                "node `{}`: {} primitives, {} unique colours, {} bright pixels",
                report.node, report.primitives, report.unique_colours, report.bright_pixels
            );
            if let Some(path) = &report.out {
                println!("wrote {path}");
            }
            if report.looks_rendered() {
                println!("ok: the frame looks like a scene");
                0
            } else {
                eprintln!(
                    "rustory-vn-editor: the frame looks blank. \
                     Render it with --out and look at it; a flat wash usually means an \
                     asset never bound, not a layout bug."
                );
                1
            }
        }
        Err(err) => {
            eprintln!("rustory-vn-editor: {err}");
            1
        }
    }
}
