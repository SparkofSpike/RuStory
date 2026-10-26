//! `rustory` — run a RuStory script, or inspect one.
//!
//! The CLI stays small on purpose: the engine is the deliverable, and a CLI that
//! grows into a build system would distract from it.

#![forbid(unsafe_code)]

mod assets;
mod driver;
mod play;
mod scene;

use std::path::{Path, PathBuf};

use rustory_core::Narrative;
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

OPTIONS for shot / play / save / resume:
    --assets <dir>     Where sprite and background PNGs live (default: assets)
    --out <file>       Output path for shot (.png) or save (.ron)
    --advance <n>      Clicks to simulate before acting (default: 0)
    --finish           resume: keep playing to an ending instead of reporting
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

struct ShotOptions {
    story: PathBuf,
    assets: PathBuf,
    out: PathBuf,
    advance: usize,
}

fn parse_shot_args(args: &[String]) -> Result<ShotOptions, String> {
    let mut story = None;
    let mut options = ShotOptions {
        story: PathBuf::new(),
        assets: PathBuf::from("assets"),
        out: PathBuf::from("shot.png"),
        advance: 0,
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

    for _ in 0..options.advance {
        if matches!(narrative.mode(), rustory_core::Mode::Finished { .. }) {
            break;
        }
        driver::click(&mut narrative, &mut registry)?;
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
