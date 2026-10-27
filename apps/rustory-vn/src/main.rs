//! `rustory` — run a RuStory script, or inspect one.
//!
//! The CLI stays small on purpose: the engine is the deliverable, and a CLI that
//! grows into a build system would distract from it.

#![forbid(unsafe_code)]

mod assets;
mod audio;
mod driver;
mod edit_cli;
mod play;
mod plugins;
mod save_screen;
mod scene;
mod slots_fs;

use std::path::{Path, PathBuf};

use rustory_vn_core::{Narrative, NodeId};
use rustory_vn_render::{Renderer, Rgba8Image};
use rustory_vn_script::ScriptError;

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
    rustory check <story.ron> [--json]  Parse and validate a script
    rustory transcript <story.ron>      Play it headlessly and print the lines
    rustory gen-assets [dir]            Write placeholder art (default: assets)
    rustory shot <story.ron> [options]  Render one frame to a PNG
    rustory play <story.ron> [options]  Play it in a window
    rustory save <story.ron> [options]  Advance, then write a save file
    rustory resume <story.ron> <save>   Load a save and report where it resumes
    rustory saves <story.ron> [--json]  List the slot grid's contents
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
    --at <node>[:<beat>|:exit]
                       Stop at a named position instead of counting clicks
    --finish           resume: keep playing to an ending instead of reporting
    --font <family>    Typeface for all text; load files from <assets>/fonts first
                       (no --font means the platform's default sans-serif)
    --language <tag>   Play a translated story in this language (e.g. en, zh-Hans);
                       without it the story's own default_language decides
    --allow-missing    Play despite keys this language does not translate, using the default
                       language's line (the key itself when there is none). Development only
                       -- `check` stays strict either way
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
            let as_json = args.iter().any(|arg| arg == "--json");
            let (flags, _) = take_plugin_flags(&args[1..]);
            check(Path::new(path), as_json, &flags)
        }
        "transcript" => transcript(&args[1..]),
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
        "saves" => saves_cmd(&args[1..]),
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
    match rustory_vn_render::probe_gpu() {
        Ok(adapter) => println!("gpu:  {adapter}"),
        Err(err) => println!("gpu:  UNAVAILABLE — {err}"),
    }
    println!("text: {}", rustory_vn_render::probe_text());

    // Audio is probed with the same "say what is missing" rule as the GPU: a silent game
    // should be explainable without reading the source.
    match rodio::DeviceSinkBuilder::open_default_sink() {
        Ok(mut device) => {
            // Probing is not playing: dropping this sink at the end of the command would
            // otherwise print rodio's "audio will stop" warning for a stream that never
            // carried a sound.
            device.log_on_drop(false);
            println!("audio: device available");
        }
        Err(err) => println!("audio: UNAVAILABLE — {err}"),
    }
    let bank = audio::AudioBank::load(Path::new("assets/audio"));
    if bank.len() == 0 {
        println!("audio files: none in assets/audio (the story can still be silent)");
    } else {
        println!("audio files: {} in assets/audio", bank.len());
    }

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

fn load(path: &Path) -> Result<rustory_vn_core::Story, String> {
    rustory_vn_script::load_story(path).map_err(|err| match err {
        ScriptError::Io { .. } => format!("cannot read {}", path.display()),
        other => other.to_string(),
    })
}

/// Load a story ready to be played: in `language` when one is named, otherwise as the story
/// asks.
///
/// The distinction matters for the editor's tools too: `check` wants the untranslated story so
/// it can report on every language, while `play` and `shot` want one language resolved.
fn load_for_playback(
    path: &Path,
    language: Option<&str>,
    missing: rustory_vn_core::MissingText,
) -> Result<rustory_vn_core::Story, String> {
    let story = load(path)?;
    let Some(language) = language else {
        // `Narrative::new` resolves through `default_language`, so an unnamed language is not
        // an omission here — it is "the story decides".
        return Ok(story);
    };
    story
        .localised_with(language, missing)
        .map_err(|err| match err {
            rustory_vn_core::CoreError::UnknownLanguage(tag) => format!(
                "no language `{tag}` in {}; it has {:?}",
                path.display(),
                story.languages()
            ),
            rustory_vn_core::CoreError::MissingTranslation { key, .. } => format!(
                "`{key}` has no {} text in {}. Pass --allow-missing to play with the default \
                 language's line instead (development only; the key is shown when there is none)",
                language,
                path.display()
            ),
            other => other.to_string(),
        })
}

/// Build the registry for `story`, loading the modules it names.
///
/// `flags` are `--plugin name=path` requests. Every caller that is about to run a story goes
/// through this, so "which systems exist" has one answer rather than one per command.
fn registry_for(
    story: &rustory_vn_core::Story,
    story_path: &Path,
    flags: &[String],
) -> Result<(rustory_vn_core::GameplayRegistry, plugins::LoadReport), String> {
    let mut wanted = std::collections::BTreeMap::new();
    for flag in flags {
        let request = plugins::PluginRequest::parse(flag)?;
        wanted.insert(request.name, request.path);
    }

    let mut registry = driver::default_registry();
    let story_dir = story_path.parent().unwrap_or(Path::new("."));
    let report = plugins::load_into(&mut registry, story, story_dir, &wanted);
    Ok((registry, report))
}

/// Collect `--plugin` arguments out of an argument list, leaving the rest.
///
/// Each command parses its own options and none of them knew about plugins; taking them out
/// first is cheaper than teaching five parsers the same flag, and it means `--plugin` works in
/// every position rather than only where someone remembered to add a branch.
fn take_plugin_flags(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut flags = Vec::new();
    let mut rest = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--plugin" {
            if let Some(value) = args.get(index + 1) {
                flags.push(value.clone());
                index += 2;
                continue;
            }
            // Left in place so the caller's own parser reports the missing value in its style.
            rest.push(args[index].clone());
            index += 1;
            continue;
        }
        rest.push(args[index].clone());
        index += 1;
    }
    (flags, rest)
}

fn check(path: &Path, as_json: bool, plugin_flags: &[String]) -> Result<(), String> {
    let story = match load(path) {
        Ok(story) => story,
        Err(err) => {
            if as_json {
                // stdout carries the document; the exit code is what a script reads.
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": false,
                        "path": path.display().to_string(),
                        "error": err,
                    })
                );
            }
            return Err(err);
        }
    };

    let choices = story
        .nodes
        .values()
        .filter(|node| matches!(node.exit, rustory_vn_core::Exit::Choose { .. }))
        .count();
    let gameplay = story
        .nodes
        .values()
        .filter(|node| matches!(node.exit, rustory_vn_core::Exit::Gameplay { .. }))
        .count();

    // Systems the story invokes but nothing registered: a load-time failure is much
    // cheaper than discovering it three scenes in. Plugins are loaded first, so a story whose
    // module is present on disk is checked against the real system rather than reported as
    // missing.
    let (mut registry, plugin_report) = registry_for(&story, path, plugin_flags)?;
    let mut missing = Vec::new();
    for system in driver::required_systems(&story) {
        if !registry.contains(&rustory_vn_core::SystemId::from(system.as_str())) {
            missing.push(system);
        }
    }

    // Every registered system parses its own config here. A typo in one used to be invisible
    // until playback reached that node, in front of a player.
    let bad_configs = registry.check_configs(&story);

    // The same walk the editor shows: one definition of "is this story sound", used by
    // the panel, `rustory edit` and this report.
    let problems = rustory_vn_edit::problems_of(&story);

    let mut mappings: Vec<(String, Vec<String>)> = Vec::new();
    for (system, outcomes) in driver::mapped_outcomes(&story) {
        mappings.push((
            system.to_string(),
            outcomes.iter().map(|key| key.to_string()).collect(),
        ));
    }

    // Translations, reported per language rather than as one "is it complete" boolean: a
    // project shipping three languages needs to know which one is short, not that one is.
    let languages: Vec<String> = story.languages().into_iter().map(str::to_owned).collect();
    let translations: Vec<serde_json::Value> = languages
        .iter()
        .map(|language| {
            let missing = story.missing_translations(language);
            serde_json::json!({
                "language": language,
                "keys": story.locales.get(language).map_or(0, std::collections::BTreeMap::len),
                "missing": missing,
            })
        })
        .collect();
    let untranslated_keys = story.used_keys().len();

    if as_json {
        println!(
            "{}",
            serde_json::json!({
                // `problems` are warnings — an unreachable draft branch is a normal state
                // while writing. A rejected config is an error: the story cannot be played
                // through, so the report says so and the exit code follows.
                "ok": bad_configs.is_empty() && plugin_report.failed.is_empty(),
                "path": path.display().to_string(),
                "nodes": story.nodes.len(),
                "beats": story.beat_count(),
                "choice_points": choices,
                "gameplay_handovers": gameplay,
                "title": story.meta.title,
                "author": story.meta.author,
                "missing_systems": missing,
                "problems": problems,
                "bad_configs": bad_configs,
                "plugins": {
                    "loaded": plugin_report.loaded.iter().map(|plugin| serde_json::json!({
                        "name": plugin.name.to_string(),
                        "path": plugin.path.display().to_string(),
                        "source": plugin.source.as_str(),
                    })).collect::<Vec<_>>(),
                    "failed": plugin_report.failed.iter().map(|(name, reason)| serde_json::json!({
                        "name": name.to_string(),
                        "reason": reason,
                    })).collect::<Vec<_>>(),
                },
                "languages": languages,
                "default_language": story.default_language,
                "text_keys": untranslated_keys,
                "translations": translations,
                "outcomes": mappings
                    .iter()
                    .map(|(system, handles)| serde_json::json!({
                        "system": system,
                        "handles": handles,
                    }))
                    .collect::<Vec<_>>(),
            })
        );
        if !bad_configs.is_empty() {
            return Err(format!(
                "{} gameplay config(s) rejected; the story cannot be played through",
                bad_configs.len()
            ));
        }
        return Ok(());
    }

    // The first line says which verdict this is. Printing `ok:` and then listing rejected
    // configs or a module that would not load would be a report that contradicts its own first
    // word.
    let failed = !bad_configs.is_empty() || !plugin_report.failed.is_empty();
    if failed {
        println!("failed: {}", path.display());
    } else {
        println!("ok: {}", path.display());
    }
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
    if !languages.is_empty() {
        println!(
            "{} text key(s), {} language(s){}",
            untranslated_keys,
            languages.len(),
            story
                .default_language
                .as_ref()
                .map(|default| format!(", default `{default}`"))
                .unwrap_or_default()
        );
        for entry in &translations {
            let language = entry["language"].as_str().unwrap_or("?");
            let missing: Vec<&str> = entry["missing"]
                .as_array()
                .map(|keys| keys.iter().filter_map(|key| key.as_str()).collect())
                .unwrap_or_default();
            if missing.is_empty() {
                println!("language {language}: complete");
            } else {
                println!("language {language}: missing {}", missing.join(", "));
            }
        }
    }
    for problem in &problems {
        println!("problem: {problem}");
    }
    for system in &missing {
        println!("warning: gameplay system `{system}` is not registered");
    }
    for line in plugin_report.summary() {
        println!("{line}");
    }
    for complaint in &bad_configs {
        println!("problem: {complaint}");
    }
    for (system, handles) in &mappings {
        println!("{system}: handles {}", handles.join(", "));
    }
    if !bad_configs.is_empty() {
        // Last, so the complaints are the final thing on screen: a report that ends with a
        // cheerful summary reads as a pass no matter what came before it.
        return Err(format!(
            "{} gameplay config(s) rejected; the story cannot be played through",
            bad_configs.len()
        ));
    }
    if let Some((name, reason)) = plugin_report.failed.first() {
        return Err(format!("plugin `{name}` could not be loaded: {reason}"));
    }
    Ok(())
}

fn transcript(args: &[String]) -> Result<(), String> {
    let (plugin_flags, args) = take_plugin_flags(args);
    let args = &args[..];
    let mut story_path: Option<PathBuf> = None;
    let mut language: Option<String> = None;
    let mut missing = rustory_vn_core::MissingText::Refuse;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--language" => {
                index += 1;
                language = Some(
                    args.get(index)
                        .ok_or("--language needs a tag such as `en` or `zh-Hans`")?
                        .clone(),
                );
            }
            "--allow-missing" => missing = rustory_vn_core::MissingText::FallBack,
            other if other.starts_with("--") => {
                return Err(format!("transcript: unknown option `{other}`"));
            }
            other => {
                if story_path.is_some() {
                    return Err(format!("transcript: unexpected argument `{other}`"));
                }
                story_path = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }

    let story_path = story_path.ok_or("transcript needs a story path")?;
    let story = load_for_playback(&story_path, language.as_deref(), missing)?;
    let (registry, report) = registry_for(&story, &story_path, &plugin_flags)?;
    if let Some((name, reason)) = report.failed.first() {
        return Err(format!("plugin `{name}` could not be loaded: {reason}"));
    }
    let lines = driver::transcript_with(story, 500, registry)?;
    for line in &lines {
        println!("{line}");
    }
    println!("---");
    println!("{} lines", lines.len());
    Ok(())
}

/// Where `--at` points.
///
/// `prologue` means "arrive at that node", which is the common case; `prologue:3` also
/// asks for the fourth beat. The beat is a runtime index, not a script index: entering
/// a node runs its leading non-blocking beats (a background, an actor) before the first
/// line appears, so `prologue:0` is usually already behind you.
///
/// `prologue:exit` is the third position and the one a gameplay author wants most: past
/// the node's last beat, so the frame shows the choice or the gameplay system the node
/// hands control to — the screen that is otherwise impossible to screenshot without a
/// window.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Position {
    Node(NodeId),
    Beat(NodeId, usize),
    Exit(NodeId),
}

impl Position {
    fn node(&self) -> &NodeId {
        match self {
            Self::Node(node) | Self::Beat(node, _) | Self::Exit(node) => node,
        }
    }
}

fn parse_at(value: &str) -> Position {
    if let Some((node, tail)) = value.rsplit_once(':') {
        if tail == "exit" {
            return Position::Exit(NodeId::from(node));
        }
        // Node ids are chosen by authors, so a colon that is not a beat or `exit` must not
        // be quietly eaten: `act:one` is one node, not node `act` beat `one`.
        if let Ok(beat) = tail.parse::<usize>() {
            return Position::Beat(NodeId::from(node), beat);
        }
    }
    Position::Node(NodeId::from(value))
}

/// Advance until the narrative is showing `target`.
///
/// Deliberately not "click n times": a click finishes a revealing line without moving
/// on, and inside a gameplay system a click plays the whole interaction to completion,
/// so click counts do not map onto positions.
fn seek(
    narrative: &mut Narrative,
    registry: &mut rustory_vn_core::GameplayRegistry,
    target: &str,
) -> Result<(), String> {
    let position = parse_at(target);
    let node = position.node().clone();
    if !narrative.story().nodes.contains_key(&node) {
        return Err(format!("no node `{node}` in this story"));
    }

    const LIMIT: usize = 1000;
    // Whether the target node has been reached at all. Without this, passing *through* an
    // earlier gameplay node would satisfy `Exit` early: the cursor is not the target yet, and
    // "we are not there" is not the same as "we have left".
    let mut reached = false;
    for _ in 0..LIMIT {
        if narrative.cursor() == &node {
            reached = true;
            match &position {
                // Arriving is the whole request when no beat was named.
                Position::Node(_) => return Ok(()),
                Position::Beat(_, beat) if narrative.beat_index() == *beat => return Ok(()),
                // Forwards only: rewinding would need a second story position, and a
                // tool that silently restarts is worse than one that says no.
                Position::Beat(_, beat) if narrative.beat_index() > *beat => {
                    return Err(format!(
                        "`{node}` is already past beat {beat} (showing {}); `--at` moves forward only",
                        narrative.beat_index()
                    ));
                }
                Position::Beat(_, _) => {}
                // Keep clicking while the node is still showing prose; stop as soon as it
                // hands over to a choice or a system, which is the frame that was asked for.
                Position::Exit(_) => {
                    if !matches!(
                        narrative.mode(),
                        rustory_vn_core::Mode::Revealing | rustory_vn_core::Mode::AwaitingAdvance
                    ) {
                        return Ok(());
                    }
                }
            }
        } else if reached && matches!(position, Position::Exit(_)) {
            // The node's exit has taken effect and the story moved on.
            return Ok(());
        }

        if matches!(narrative.mode(), rustory_vn_core::Mode::Finished { .. }) {
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
    /// Which language to play a translated story in. Absent means the story's own default.
    language: Option<String>,
    /// What to do about a key that language does not translate.
    missing: rustory_vn_core::MissingText,
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
        language: None,
        missing: rustory_vn_core::MissingText::Refuse,
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
            "--language" => {
                index += 1;
                options.language = Some(
                    args.get(index)
                        .ok_or("--language needs a tag such as `en` or `zh-Hans`")?
                        .clone(),
                );
            }
            "--allow-missing" => options.missing = rustory_vn_core::MissingText::FallBack,
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
    let (plugin_flags, args) = take_plugin_flags(args);
    let options = parse_shot_args(&args)?;
    let story = load_for_playback(&options.story, options.language.as_deref(), options.missing)?;
    let (mut registry, report) = registry_for(&story, &options.story, &plugin_flags)?;
    let mut narrative = Narrative::new(story).map_err(|err| err.to_string())?;

    if let Some((name, reason)) = report.failed.first() {
        return Err(format!("plugin `{name}` could not be loaded: {reason}"));
    }

    if let Some(target) = &options.at {
        if options.advance > 0 {
            return Err("--at and --advance both position the frame; use one".into());
        }
        seek(&mut narrative, &mut registry, target)?;
    } else {
        for _ in 0..options.advance {
            if matches!(narrative.mode(), rustory_vn_core::Mode::Finished { .. }) {
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
    if matches!(narrative.mode(), rustory_vn_core::Mode::Suspended { .. }) {
        let _ = narrative.step_gameplay(&mut registry, 0, &[]);
    }

    let mut renderer = Renderer::new_headless(WIDTH, HEIGHT).map_err(|err| err.to_string())?;
    let bank = driver::load_textures(&mut renderer, &options.assets);
    let loaded = driver::load_fonts(&mut renderer, &options.assets);
    if let Some(family) = driver::choose_font(&mut renderer, options.font.as_deref(), &loaded)? {
        println!("font: {family}");
    }

    let view = narrative.gameplay_view(&registry);
    let frame = driver::build_frame(&narrative, &bank, view.as_ref(), None);

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
    let (plugin_flags, args) = take_plugin_flags(args);
    let args = &args[..];
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
    let (mut registry, report) = registry_for(&story, &story_path, &plugin_flags)?;
    let mut narrative = Narrative::new(story).map_err(|err| err.to_string())?;

    if let Some((name, reason)) = report.failed.first() {
        return Err(format!("plugin `{name}` could not be loaded: {reason}"));
    }

    for _ in 0..advance {
        if matches!(narrative.mode(), rustory_vn_core::Mode::Finished { .. }) {
            break;
        }
        driver::click(&mut narrative, &mut registry)?;
    }

    // A script that reaches a gameplay handover leaves the mode suspended but the
    // system un-begun: `begin` runs on the first step. Saving in that window would
    // record "no state" for a system that is about to have some. A zero-length step
    // performs the handover without consuming the clock.
    if matches!(narrative.mode(), rustory_vn_core::Mode::Suspended { .. }) {
        narrative
            .step_gameplay(&mut registry, 0, &[])
            .map_err(|err| err.to_string())?;
    }

    let save = narrative.save(&registry).map_err(|err| err.to_string())?;
    let text = save.to_ron().map_err(|err| err.to_string())?;

    // Create the parent directory: `--out stories/saves/slot00.ron` on a fresh project used to
    // fail with "no such path", which is a bad first five minutes for a documented command.
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
    }
    std::fs::write(&out, text).map_err(|err| format!("cannot write {}: {err}", out.display()))?;

    println!(
        "wrote {} at node `{}` ({})",
        out.display(),
        narrative.cursor(),
        driver::describe_mode(&narrative)
    );
    Ok(())
}

/// List the save slots a story has, in layout order.
///
/// The agent-facing half of the save screen: a program cannot look at a grid, and "which slot was
/// I on" is exactly the question a resumed session asks. It shares `slots_fs` with the window, so
/// the two cannot disagree about what a slot contains.
fn saves_cmd(args: &[String]) -> Result<(), String> {
    let as_json = args.iter().any(|arg| arg == "--json");
    // `--delete <slot>` removes one file. The screen has no delete button yet, and a damaged slot
    // is precisely the thing a player needs to get rid of, so the operation exists here.
    let mut delete_slot: Option<String> = None;
    let mut shot_out: Option<String> = None;
    let mut mode = save_screen::ScreenMode::Save;
    let mut index_arg = 0;
    while index_arg < args.len() {
        match args[index_arg].as_str() {
            "--delete" => {
                delete_slot = Some(
                    args.get(index_arg + 1)
                        .ok_or("--delete needs a slot label such as `1` or `quick`")?
                        .clone(),
                );
                index_arg += 2;
                continue;
            }
            "--shot" => {
                shot_out = Some(
                    args.get(index_arg + 1)
                        .ok_or("--shot needs an output path")?
                        .clone(),
                );
                index_arg += 2;
                continue;
            }
            "--mode" => {
                let value = args
                    .get(index_arg + 1)
                    .ok_or("--mode needs `save` or `load`")?;
                mode = match value.as_str() {
                    "save" => save_screen::ScreenMode::Save,
                    "load" => save_screen::ScreenMode::Load,
                    other => return Err(format!("--mode expects `save` or `load`, got `{other}`")),
                };
                index_arg += 2;
                continue;
            }
            _ => {}
        }
        index_arg += 1;
    }

    let dir = args
        .iter()
        .enumerate()
        .filter(|(position, _)| {
            // Skip the value that follows `--delete`.
            let previous = position.checked_sub(1).and_then(|at| args.get(at));
            !args[*position].starts_with("--") && previous.is_none_or(|prev| prev != "--delete")
        })
        .map(|(_, arg)| arg)
        .find(|arg| !arg.starts_with("--"))
        .ok_or("saves needs a story path")?;

    let story_path = PathBuf::from(dir);
    let story = load(&story_path)?;
    let layout = rustory_vn_core::SlotLayout::beside(&story_path);

    if let Some(label) = delete_slot {
        let slot = layout
            .slots()
            .find(|slot| slot.label() == label || slot.stem() == label)
            .ok_or_else(|| {
                format!(
                    "no slot `{label}`; this layout has {:?}",
                    layout.slots().map(|slot| slot.label()).collect::<Vec<_>>()
                )
            })?;
        slots_fs::delete(&layout, slot).map_err(|err| err.to_string())?;
        if as_json {
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "deleted": slot.label(),
                    "path": layout.path(slot).display().to_string(),
                })
            );
        } else {
            println!("deleted slot {}", slot.label());
        }
        return Ok(());
    }

    let index = slots_fs::read_all_with_times(&layout, &story);

    if as_json {
        let slots: Vec<serde_json::Value> = layout
            .slots()
            .map(|slot| {
                let summary = index.get(slot);
                serde_json::json!({
                    "slot": slot.label(),
                    "stem": slot.stem(),
                    "path": layout.path(slot).display().to_string(),
                    "empty": summary.is_none_or(rustory_vn_core::SlotSummary::is_empty),
                    "title": summary.and_then(|s| s.title.clone()),
                    "node": summary.and_then(|s| s.node.as_ref().map(ToString::to_string)),
                    "language": summary.and_then(|s| s.language.clone()),
                    "line": summary.and_then(|s| s.line.clone()),
                    "saved_at_ms": summary.and_then(|s| s.saved_at_ms),
                    "problem": summary.and_then(|s| s.problem.clone()),
                })
            })
            .collect();

        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "path": story_path.display().to_string(),
                "dir": layout.dir().display().to_string(),
                "occupied": index.occupied(),
                "newest": index.newest().map(|summary| summary.slot.label()),
                "slots": slots,
            })
        );
        return Ok(());
    }

    println!("{}", layout.dir().display());
    for line in index.describe_all(&layout) {
        println!("{line}");
    }
    if let Some(newest) = index.newest() {
        println!("newest: {}", newest.slot.label());
    }

    // `--shot <png> --mode save|load` renders the screen itself. Not decoration: it is how the
    // screen can be looked at without a window, which is what makes it reviewable and what the
    // screenshots in the README come from.
    if let Some(out) = shot_out {
        render_screen(&story_path, &story, &layout, &index, mode, Path::new(&out))?;
    }
    Ok(())
}

/// Render the save screen to a PNG.
///
/// Uses the same `GameplayView` the window paints, so what this writes is what a player sees —
/// the alternative, a separate preview drawing, is how a screenshot stops being evidence.
fn render_screen(
    story_path: &Path,
    story: &rustory_vn_core::Story,
    layout: &rustory_vn_core::SlotLayout,
    index: &rustory_vn_core::SlotIndex,
    mode: save_screen::ScreenMode,
    out: &Path,
) -> Result<(), String> {
    use save_screen::SaveScreen as _;

    let mut renderer = Renderer::new_headless(WIDTH, HEIGHT).map_err(|err| err.to_string())?;
    let bank = driver::load_textures(&mut renderer, Path::new("assets"));
    let _ = driver::load_fonts(&mut renderer, Path::new("assets"));

    // A narrative is needed only to draw the story behind the screen, and there is none: the
    // screen covers the whole box. So the frame is built from the screen's view over an empty
    // narrative, which is what the window does too.
    let _ = story_path;
    let view = save_screen::GridScreen.view(index, layout, mode);
    let narrative = Narrative::new(story.clone()).map_err(|err| err.to_string())?;
    let frame = driver::build_frame(&narrative, &bank, Some(&view), None);

    let image = renderer
        .render_offscreen(&frame)
        .map_err(|err| err.to_string())?;
    image.save_png(out).map_err(|err| err.to_string())?;

    println!(
        "wrote {} ({}x{}, {} primitives, {})",
        out.display(),
        image.width(),
        image.height(),
        frame.primitive_count(),
        match mode {
            save_screen::ScreenMode::Save => "save screen",
            save_screen::ScreenMode::Load => "load screen",
        }
    );
    Ok(())
}

/// Load a save and report where it resumes; with `--finish`, keep playing.
fn resume_cmd(args: &[String]) -> Result<(), String> {
    let (plugin_flags, args) = take_plugin_flags(args);
    let args = &args[..];
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
    let save = rustory_vn_core::SaveState::from_ron(&text).map_err(|err| err.to_string())?;

    // A save may have been taken while a plugin had control, so the module has to be loaded
    // before `from_save` asks it to restore its own state.
    let (mut registry, report) = registry_for(&story, story_path, &plugin_flags)?;
    if let Some((name, reason)) = report.failed.first() {
        return Err(format!("plugin `{name}` could not be loaded: {reason}"));
    }
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
            if matches!(narrative.mode(), rustory_vn_core::Mode::Finished { .. }) {
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

    // Accepting both would make the order of application a guess. Every early return below
    // announces itself in the requested format: a caller that asked for JSON and got a
    // sentence on stderr has to parse prose to find out what went wrong.
    let ops = match (single, batch) {
        (Some(_), Some(_)) => {
            let message = "use either --op or --ops, not both".to_owned();
            edit_cli::emit_failure_json(as_json, "bad_arguments", &message);
            return Err(message);
        }
        (None, None) => {
            let message = "edit needs --op <json> or --ops <file>".to_owned();
            edit_cli::emit_failure_json(as_json, "bad_arguments", &message);
            return Err(message);
        }
        (Some(json), None) => match edit_cli::parse_op(&json) {
            Ok(op) => vec![op],
            Err(message) => {
                edit_cli::emit_failure_json(as_json, "invalid_operation", &message);
                return Err(message);
            }
        },
        (None, Some(path)) => {
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) => {
                    let message = format!("cannot read {}: {err}", path.display());
                    edit_cli::emit_failure_json(as_json, "cannot_open", &message);
                    return Err(message);
                }
            };
            match edit_cli::parse_ops(&text) {
                Ok(ops) => ops,
                Err(message) => {
                    edit_cli::emit_failure_json(as_json, "invalid_operation", &message);
                    return Err(message);
                }
            }
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
        assert_eq!(
            parse_at("prologue"),
            Position::Node(NodeId::from("prologue"))
        );
    }

    #[test]
    fn a_colon_names_a_beat_inside_the_node() {
        assert_eq!(
            parse_at("prologue:3"),
            Position::Beat(NodeId::from("prologue"), 3)
        );
    }

    #[test]
    fn exit_names_the_screen_the_node_hands_over_to() {
        // The position a gameplay author needs: past the last beat, so the frame is the
        // system's own screen rather than the line that preceded it.
        assert_eq!(
            parse_at("honest:exit"),
            Position::Exit(NodeId::from("honest"))
        );
    }

    #[test]
    fn a_colon_without_a_number_stays_part_of_the_node_name() {
        // Node ids are chosen by authors, so a colon that is not a beat must not be
        // quietly eaten: `act:one` is one node, not node `act` beat `one`.
        assert_eq!(parse_at("act:one"), Position::Node(NodeId::from("act:one")));
    }
}
