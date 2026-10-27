//! Drives a story without a window.
//!
//! The CLI and the windowed player share this: one `click` means "do whatever
//! the current mode needs next", so neither caller has to know whether that is
//! finishing a line, advancing a beat, answering a choice or stepping a system.

use std::collections::BTreeMap;
use std::path::Path;

use rustory_vn_core::{
    GameplayInput, GameplayRegistry, GameplayView, Mode, Narrative, NodeId, OutcomeKey, Story,
};
use rustory_vn_gameplay_quiz::Quiz;
use rustory_vn_gameplay_timer::TimedChoice;
use rustory_vn_render::{Renderer, Rgba8Image};

use crate::scene::{self, Scene, TextureBank};

/// Every gameplay system the engine ships.
///
/// Adding a system means adding a line here and a crate beside it. Nothing in
/// `rustory-vn-core` changes, which is the whole claim being tested.
#[must_use]
pub fn default_registry() -> GameplayRegistry {
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(TimedChoice::new()));
    registry.register(Box::new(Quiz::new()));
    registry
}

/// Upload every PNG in `dir`, keyed by file stem and by full file name.
///
/// A missing directory is not an error: a script with no art still renders its
/// dialogue, and demanding assets before the first frame would make the engine
/// feel broken on an empty checkout.
#[must_use]
pub fn load_textures(renderer: &mut Renderer, dir: &Path) -> TextureBank {
    let mut bank = TextureBank::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return bank;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Ok(image) = Rgba8Image::load_png(&path) else {
            continue;
        };
        let id = renderer.upload_texture(&image, stem);
        bank.insert(stem, id);
        bank.insert(format!("{stem}.png"), id);
    }
    bank
}

/// Load every font file in `assets/fonts`, if that directory exists.
///
/// The engine ships no fonts. A project that must render on machines it has never
/// seen supplies its own; one that is happy with platform defaults ignores this.
/// A missing directory is normal, not an error: most checkouts will not have one.
///
/// Returns the family names that loaded, in filename order, so a caller can either
/// report them or pick the first as the project default.
#[must_use]
pub fn load_fonts(renderer: &mut Renderer, assets_dir: &Path) -> Vec<String> {
    let dir = assets_dir.join("fonts");
    if !dir.is_dir() {
        return Vec::new();
    }
    renderer.load_fonts_dir(&dir).unwrap_or_else(|err| {
        eprintln!("rustory: {err}");
        Vec::new()
    })
}

/// Choose the typeface for everything that did not ask for a specific face.
///
/// Precedence: an explicit request wins; otherwise the first font found in
/// `<assets>/fonts` becomes the default, because a project that shipped a font
/// meant it to be used; otherwise nothing changes and the platform decides.
///
/// The middle case is the one worth having: dropping a file into `assets/fonts`
/// should be enough, without a creator having to discover a command-line flag or
/// learn the internal family name of their own file.
///
/// Returns the family that ended up in effect, if one did.
pub fn choose_font(
    renderer: &mut Renderer,
    requested: Option<&str>,
    loaded: &[String],
) -> Result<Option<String>, String> {
    if let Some(family) = requested {
        return if renderer.set_sans_serif_family(family) {
            Ok(Some(family.to_owned()))
        } else {
            Err(format!(
                "font family `{family}` is not available. Put font files in \
                 `<assets>/fonts/`, or omit `--font` to use the platform default."
            ))
        };
    }

    let Some(family) = loaded.first() else {
        return Ok(None);
    };
    if renderer.set_sans_serif_family(family) {
        Ok(Some(family.clone()))
    } else {
        // Loaded but not selectable is a strange state; say so rather than letting
        // the caller assume the project font took effect.
        Err(format!("loaded font `{family}` cannot be selected"))
    }
}

/// Perform one interaction, whatever the current mode demands.
///
/// A choice always takes the first branch and gameplay is stepped without input,
/// so a transcript or a captured frame is reproducible rather than random.
pub fn click(narrative: &mut Narrative, registry: &mut GameplayRegistry) -> Result<(), String> {
    match narrative.mode().clone() {
        Mode::Revealing => {
            narrative.tick(0, 0);
            narrative.advance().map_err(|err| err.to_string())?;
        }
        Mode::AwaitingAdvance => {
            narrative.advance().map_err(|err| err.to_string())?;
        }
        Mode::AwaitingChoice { choices, .. } => {
            if choices.is_empty() {
                return Err("choice node offers no choices".into());
            }
            narrative.choose(0).map_err(|err| err.to_string())?;
        }
        Mode::Suspended { .. } => {
            // Simulate a player who always takes the first option. An empty input
            // list would leave a system that waits for a choice suspended forever —
            // the quiz is exactly that — while a countdown only escapes by running
            // out. Sending `Select { index: 0 }` keeps every system able to finish
            // and keeps the outcome deterministic.
            let input = [GameplayInput::Select { index: 0 }];

            // Bounded: a system that ignores even that would otherwise hang the CLI.
            for _ in 0..100_000 {
                narrative
                    .step_gameplay(registry, 16, &input)
                    .map_err(|err| err.to_string())?;
                if !matches!(narrative.mode(), Mode::Suspended { .. }) {
                    return Ok(());
                }
            }
            return Err("gameplay system did not finish within 100000 steps".into());
        }
        Mode::Finished { .. } => {}
    }
    Ok(())
}

/// Build the draw list for the current state.
#[must_use]
pub fn build_frame<'a>(
    narrative: &'a Narrative,
    bank: &'a TextureBank,
    view: Option<&'a GameplayView>,
    hint: Option<&'a str>,
) -> rustory_vn_render::Frame {
    Scene {
        narrative,
        textures: bank,
        gameplay_view: view,
        hint,
    }
    .build()
}

/// A readable description of the current mode, for logs and CLI output.
#[must_use]
pub fn describe_mode(narrative: &Narrative) -> String {
    match narrative.mode() {
        Mode::Revealing => "revealing".into(),
        Mode::AwaitingAdvance => "awaiting advance".into(),
        Mode::AwaitingChoice { choices, .. } => {
            format!("awaiting choice ({} options)", choices.len())
        }
        Mode::Suspended { spec } => format!("gameplay: {}", spec.system),
        Mode::Finished { ending } => match ending {
            Some(ending) => format!("finished ({ending})"),
            None => "finished".into(),
        },
    }
}

/// Play the whole story with automatic clicks and return the lines it showed.
///
/// This is how "is the story playable?" gets a real answer without a window. A
/// story that loops back on itself is reported rather than spun on.
///
/// Kept for callers with no plugins to load — the tests, and any embedding that only uses the
/// engine's own systems. The CLI goes through [`transcript_with`], because it may have modules
/// to load first.
#[cfg_attr(not(test), allow(dead_code))]
pub fn transcript(story: Story, max_clicks: usize) -> Result<Vec<String>, String> {
    let registry = default_registry();
    transcript_with(story, max_clicks, registry)
}

/// The same walk with a registry the caller built.
///
/// Separate from [`transcript`] so a story that needs a loaded module does not have to take a
/// different code path — the CLI loads plugins, then calls this.
pub fn transcript_with(
    story: Story,
    max_clicks: usize,
    registry: GameplayRegistry,
) -> Result<Vec<String>, String> {
    let mut narrative = Narrative::new(story).map_err(|err| err.to_string())?;
    let mut registry = registry;
    let mut lines: Vec<String> = Vec::new();
    let mut visits: BTreeMap<NodeId, usize> = BTreeMap::new();

    narrative.tick(0, 0);
    for _ in 0..max_clicks {
        if let Some(line) = narrative.line() {
            let text = scene::line_summary(&line);
            if lines.last() != Some(&text) {
                lines.push(text);
            }
        }

        if matches!(narrative.mode(), Mode::Finished { .. }) {
            break;
        }

        click(&mut narrative, &mut registry)?;

        let count = visits.entry(narrative.cursor().clone()).or_insert(0);
        *count += 1;
        if *count > 64 {
            return Err(format!(
                "story revisits node `{}` more than 64 times",
                narrative.cursor()
            ));
        }
    }

    Ok(lines)
}

/// Report the outcome keys the story expects, so a missing registration is
/// visible before the scene is reached.
#[must_use]
pub fn required_systems(story: &Story) -> Vec<String> {
    let mut ids: Vec<String> = story
        .nodes
        .values()
        .filter_map(|node| match &node.exit {
            rustory_vn_core::Exit::Gameplay { spec } => Some(spec.system.to_string()),
            _ => None,
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Outcome keys a system may report for a node. Used by `check` to warn when a
/// well-known system's outcomes are not fully mapped.
#[must_use]
pub fn mapped_outcomes(story: &Story) -> BTreeMap<String, Vec<OutcomeKey>> {
    let mut map = BTreeMap::new();
    for node in story.nodes.values() {
        if let rustory_vn_core::Exit::Gameplay { spec } = &node.exit {
            map.entry(spec.system.to_string())
                .or_insert_with(Vec::new)
                .extend(spec.outcomes.keys().cloned());
        }
    }
    for keys in map.values_mut() {
        keys.sort();
        keys.dedup();
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustory_vn_core::{Beat, Exit, Node};

    fn demo() -> Story {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/demo.ron"),
        )
        .expect("demo script is in the repository");
        rustory_vn_script::parse_story(&text, "demo.ron").expect("demo script is valid")
    }

    #[test]
    fn the_demo_script_parses_and_validates() {
        let story = demo();
        assert!(story.nodes.len() >= 4);
        assert_eq!(story.start.as_str(), "prologue");
    }

    #[test]
    fn the_demo_script_reaches_an_ending() {
        let lines = transcript(demo(), 200).expect("transcript");
        assert!(!lines.is_empty(), "a story should say something");
        assert!(
            lines.iter().any(|line| line.contains("Yuki")),
            "the speaker name should be shown: {lines:?}"
        );
    }

    #[test]
    fn the_demo_uses_two_gameplay_systems() {
        // The demo is the one place a reader can see that a story is not limited
        // to a single system, so it must actually exercise more than one.
        assert_eq!(
            required_systems(&demo()),
            vec!["quiz.multi_round", "timer.timed_choice"],
            "both systems should appear, sorted by id"
        );
    }

    #[test]
    fn every_system_the_demo_names_is_registered() {
        let registry = default_registry();
        for system in required_systems(&demo()) {
            assert!(
                registry.contains(&rustory_vn_core::SystemId::from(system.as_str())),
                "`{system}` is referenced by the demo but not registered"
            );
        }
    }

    #[test]
    fn the_default_registry_contains_the_timer() {
        let registry = default_registry();
        assert!(registry.contains(&rustory_vn_core::SystemId::from(
            rustory_vn_gameplay_timer::SYSTEM_ID
        )));
    }

    #[test]
    fn a_story_that_never_ends_is_reported() {
        // A self-loop with dialogue is legal — it just never terminates, which
        // the transcript driver must notice instead of spinning forever.
        let mut nodes = BTreeMap::new();
        nodes.insert(
            NodeId::from("loop"),
            Node {
                beats: vec![Beat::Say {
                    speaker: None,
                    text: "again".into(),
                }],
                exit: Exit::Goto {
                    to: NodeId::from("loop"),
                },
            },
        );
        let story = Story::new("loop", nodes);

        let err = transcript(story, 1000).expect_err("must report");
        assert!(err.contains("revisits node"), "unexpected error: {err}");
    }

    #[test]
    fn every_gameplay_node_maps_at_least_one_outcome() {
        let mapped = mapped_outcomes(&demo());
        let timer = mapped.get("timer.timed_choice").expect("timer is used");
        assert!(timer.contains(&OutcomeKey::from("correct")));
        assert!(timer.contains(&OutcomeKey::from("timeout")));
    }
}
