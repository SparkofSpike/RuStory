//! Loading gameplay modules named by a story.
//!
//! The contract is `GameplaySystem`; a WebAssembly module is one way to satisfy it. This module
//! answers a narrower question: a script says `system: "wasm.puzzle"`, so where does the code
//! come from?
//!
//! Three sources, in this order, and the order is the whole design:
//!
//! 1. **A flag** — `--plugin wasm.puzzle=puzzles/slide.wasm`. Wins over everything, because a
//!    person typing it is trying to run a specific build.
//! 2. **The story's own `plugins` map** — the path the author recorded. This is what makes a
//!    cloned repository playable without a command line full of flags.
//! 3. **A `plugins/` directory beside the story** — `plugins/wasm.puzzle.wasm` or
//!    `plugins/wasm.puzzle/plugin.wasm`. A convention, not a requirement; it exists so the common
//!    case needs no configuration at all.
//!
//! A name that resolves to nothing is not an error *here*. A story may legitimately name a
//! compiled-in system, and this module has no way to tell the two apart. The caller checks what
//! is still missing after loading, and `rustory check` reports it before anything plays.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rustory_vn_core::{GameplayRegistry, Story, SystemId};
use rustory_vn_gameplay_wasm::WasmGameplay;

/// One module a caller pinned to a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRequest {
    pub name: SystemId,
    pub path: PathBuf,
}

impl PluginRequest {
    /// Parse `name=path`.
    ///
    /// The name is everything before the first `=`, so a path may contain one; the reverse
    /// would make `--plugin a=b=c` ambiguous, and a path with `=` in it is far more likely than
    /// a system id with one.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let (name, path) = spec
            .split_once('=')
            .ok_or_else(|| format!("--plugin expects `name=path`, got `{spec}`"))?;
        if name.trim().is_empty() || path.trim().is_empty() {
            return Err(format!("--plugin expects `name=path`, got `{spec}`"));
        }
        Ok(Self {
            name: SystemId::from(name.trim()),
            path: PathBuf::from(path.trim()),
        })
    }
}

/// A module that was found and loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedPlugin {
    pub name: SystemId,
    pub path: PathBuf,
    /// Where the path came from, because "it loaded the wrong one" is the failure this
    /// description exists to make diagnosable.
    pub source: PluginSource,
}

/// Which of the three sources produced a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSource {
    /// `--plugin name=path`
    Flag,
    /// The story's `plugins` map.
    Story,
    /// A file in `plugins/` beside the story.
    Directory,
}

impl PluginSource {
    /// A short label for a report line.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Story => "story",
            Self::Directory => "directory",
        }
    }
}

/// The result of loading: what went in, and what could not.
#[derive(Debug, Default)]
pub struct LoadReport {
    pub loaded: Vec<LoadedPlugin>,
    /// Name and reason, one line each. Not fatal on its own — see the module docs.
    pub failed: Vec<(SystemId, String)>,
}

impl LoadReport {
    /// One line per module, for a CLI that wants to say what it did.
    #[must_use]
    pub fn summary(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .loaded
            .iter()
            .map(|plugin| {
                format!(
                    "plugin {}: {} ({})",
                    plugin.name,
                    plugin.path.display(),
                    plugin.source.as_str()
                )
            })
            .collect();
        for (name, reason) in &self.failed {
            lines.push(format!("plugin {name}: not loaded — {reason}"));
        }
        lines
    }
}

/// Every name the story hands control to.
///
/// The union of what the script invokes and what it declares: a script that declares a module but
/// no longer uses it should not load it, and a script that uses one without declaring it should
/// still be able to find it in `plugins/`.
#[must_use]
pub fn wanted(story: &Story) -> Vec<SystemId> {
    let mut names: Vec<SystemId> = story
        .nodes
        .values()
        .filter_map(|node| match &node.exit {
            rustory_vn_core::Exit::Gameplay { spec } => Some(spec.system.clone()),
            _ => None,
        })
        .collect();
    for name in story.plugins.keys() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Where a module for `name` lives, if anything can be found.
///
/// `story_dir` is where the story file sits, which is what makes the `plugins/` convention
/// relative to the project rather than to the working directory — a story opened from elsewhere
/// must still find its own modules.
#[must_use]
pub fn resolve(
    name: &SystemId,
    story: &Story,
    story_dir: &Path,
    explicit: &BTreeMap<SystemId, PathBuf>,
) -> Option<(PathBuf, PluginSource)> {
    if let Some(path) = explicit.get(name) {
        return Some((path.clone(), PluginSource::Flag));
    }
    if let Some(path) = story.plugins.get(name) {
        // A relative path in the story is relative to the story. Absolute paths are taken as
        // written, which is what an author who wrote one meant.
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            story_dir.join(path)
        };
        return Some((path, PluginSource::Story));
    }

    // The convention has two homes, and both are tried because both are what projects actually
    // do: `plugins/` beside the story (one directory per chapter of modules) and `plugins/` one
    // level up (one directory for the project). The report names the path that won, so "which
    // one did it load" never needs reconstructing.
    let mut directories = vec![story_dir.join("plugins")];
    if let Some(parent) = story_dir.parent()
        && parent != story_dir
    {
        directories.push(parent.join("plugins"));
    }

    for dir in directories {
        let direct = dir.join(format!("{name}.wasm"));
        if direct.is_file() {
            return Some((direct, PluginSource::Directory));
        }
        let nested = dir.join(name.as_str()).join("plugin.wasm");
        if nested.is_file() {
            return Some((nested, PluginSource::Directory));
        }
    }
    None
}

/// Load every module the story needs into `registry`.
///
/// Modules already registered under a name are left alone: a compiled-in system wins over a file
/// with the same id, because the compiled-in one is the one the engine promises.
pub fn load_into(
    registry: &mut GameplayRegistry,
    story: &Story,
    story_dir: &Path,
    explicit: &BTreeMap<SystemId, PathBuf>,
) -> LoadReport {
    let mut report = LoadReport::default();

    for name in wanted(story) {
        if registry.contains(&name) {
            continue;
        }
        let Some((path, source)) = resolve(&name, story, story_dir, explicit) else {
            continue;
        };

        match WasmGameplay::open(name.clone(), &path) {
            Ok(system) => {
                registry.register(Box::new(system));
                report.loaded.push(LoadedPlugin { name, path, source });
            }
            Err(err) => report.failed.push((name, err.to_string())),
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustory_vn_core::{Exit, GameplaySpec, Node, NodeId};

    fn story_with_plugins(plugins: &[(&str, &str)], systems: &[&str]) -> Story {
        let mut nodes = BTreeMap::new();
        for (index, system) in systems.iter().enumerate() {
            nodes.insert(
                NodeId::from(format!("n{index}")),
                Node {
                    beats: Vec::new(),
                    exit: Exit::Gameplay {
                        spec: GameplaySpec {
                            system: SystemId::from(*system),
                            config: None,
                            outcomes: BTreeMap::new(),
                        },
                    },
                },
            );
        }
        nodes.insert(
            NodeId::from("end"),
            Node {
                beats: Vec::new(),
                exit: Exit::End { ending: None },
            },
        );

        let mut story = Story::new(systems.first().copied().unwrap_or("end"), nodes);
        story.plugins = plugins
            .iter()
            .map(|(name, path)| (SystemId::from(*name), (*path).to_owned()))
            .collect();
        story
    }

    #[test]
    fn a_plugin_flag_is_parsed_at_the_first_equals() {
        // A path may contain `=`; a system id will not. Splitting the other way would make
        // `--plugin a=b=c` ambiguous.
        let request = PluginRequest::parse("wasm.puzzle=dir/slide=2.wasm").expect("parses");
        assert_eq!(request.name, SystemId::from("wasm.puzzle"));
        assert_eq!(request.path, PathBuf::from("dir/slide=2.wasm"));
    }

    #[test]
    fn a_plugin_flag_without_a_path_is_refused() {
        for spec in ["wasm.puzzle", "=x.wasm", "wasm.puzzle=", ""] {
            assert!(
                PluginRequest::parse(spec).is_err(),
                "`{spec}` must not parse"
            );
        }
    }

    #[test]
    fn wanted_covers_what_the_script_invokes_and_what_it_declares() {
        let story = story_with_plugins(
            &[("wasm.unused", "a.wasm"), ("wasm.declared", "b.wasm")],
            &["wasm.used"],
        );
        let names: Vec<String> = wanted(&story)
            .into_iter()
            .map(|id| id.to_string())
            .collect();

        assert_eq!(
            names,
            ["wasm.declared", "wasm.unused", "wasm.used"],
            "sorted and deduplicated, with declared-but-unused modules included"
        );
    }

    #[test]
    fn a_flag_beats_the_story_and_the_directory() {
        let story = story_with_plugins(&[("wasm.puzzle", "story.wasm")], &["wasm.puzzle"]);
        let explicit =
            BTreeMap::from([(SystemId::from("wasm.puzzle"), PathBuf::from("flag.wasm"))]);

        let (path, source) = resolve(
            &SystemId::from("wasm.puzzle"),
            &story,
            Path::new("/proj"),
            &explicit,
        )
        .expect("resolves");

        assert_eq!(path, PathBuf::from("flag.wasm"));
        assert_eq!(source, PluginSource::Flag);
    }

    #[test]
    fn a_story_path_is_relative_to_the_story() {
        // The story may be opened from anywhere; its modules travel with it.
        let story = story_with_plugins(&[("wasm.puzzle", "mods/slide.wasm")], &["wasm.puzzle"]);
        let (path, source) = resolve(
            &SystemId::from("wasm.puzzle"),
            &story,
            Path::new("/proj"),
            &BTreeMap::new(),
        )
        .expect("resolves");

        assert_eq!(path, PathBuf::from("/proj").join("mods/slide.wasm"));
        assert_eq!(source, PluginSource::Story);
    }

    #[test]
    fn an_absolute_story_path_is_taken_as_written() {
        let absolute = if cfg!(windows) {
            "C:/mods/slide.wasm"
        } else {
            "/mods/slide.wasm"
        };
        let story = story_with_plugins(&[("wasm.puzzle", absolute)], &["wasm.puzzle"]);

        let (path, _) = resolve(
            &SystemId::from("wasm.puzzle"),
            &story,
            Path::new("/proj"),
            &BTreeMap::new(),
        )
        .expect("resolves");
        assert_eq!(path, PathBuf::from(absolute));
    }

    #[test]
    fn a_name_nothing_provides_resolves_to_nothing() {
        // Not an error: the name may belong to a compiled-in system, and this layer cannot tell.
        let story = story_with_plugins(&[], &["timer.timed_choice"]);
        assert!(
            resolve(
                &SystemId::from("timer.timed_choice"),
                &story,
                Path::new("/proj"),
                &BTreeMap::new()
            )
            .is_none()
        );
    }

    #[test]
    fn a_missing_module_is_reported_rather_than_skipped_silently() {
        // The path is declared, so the author meant it to exist. Loading nothing would leave the
        // story unplayable with no explanation.
        let story = story_with_plugins(&[("wasm.puzzle", "nowhere.wasm")], &["wasm.puzzle"]);
        let mut registry = GameplayRegistry::new();

        let report = load_into(
            &mut registry,
            &story,
            &std::env::temp_dir(),
            &BTreeMap::new(),
        );

        assert_eq!(report.loaded.len(), 0);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, SystemId::from("wasm.puzzle"));
        assert!(
            report.failed[0].1.contains("cannot read"),
            "got {}",
            report.failed[0].1
        );
    }

    #[test]
    fn a_compiled_in_system_is_not_replaced_by_a_file() {
        // The engine's own systems are a promise; a module with the same id is not entitled to
        // take the name over.
        use rustory_vn_gameplay_timer::TimedChoice;

        let story = story_with_plugins(
            &[("timer.timed_choice", "evil.wasm")],
            &["timer.timed_choice"],
        );
        let mut registry = GameplayRegistry::new();
        registry.register(Box::new(TimedChoice::new()));

        let report = load_into(
            &mut registry,
            &story,
            &std::env::temp_dir(),
            &BTreeMap::new(),
        );

        assert!(report.loaded.is_empty());
        assert!(report.failed.is_empty(), "it was skipped, not attempted");
    }
}
