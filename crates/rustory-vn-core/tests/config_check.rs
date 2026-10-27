//! Checking every gameplay config without playing the story.
//!
//! The theme: a bad config should be a load-time complaint, not a mid-scene crash. A config is
//! opaque to the core, so only the system that owns it can say whether it is valid — which is why
//! the check has to run through the registry rather than parse anything here.

use std::collections::BTreeMap;

use rustory_vn_core::prelude::*;

/// Accepts a config containing `ok: true` and rejects everything else, including absence.
struct Picky;

#[derive(serde::Deserialize)]
struct PickyConfig {
    ok: bool,
}

impl GameplaySystem for Picky {
    fn id(&self) -> SystemId {
        SystemId::from("test.picky")
    }

    fn begin(
        &mut self,
        config: Option<&ron::Value>,
        _narrative: &NarrativeSnapshot,
    ) -> Result<(), CoreError> {
        let value = config.ok_or_else(|| CoreError::GameplayFailed {
            system: "test.picky".to_owned(),
            message: "a config is required".to_owned(),
        })?;
        let parsed: PickyConfig =
            value
                .clone()
                .into_rust()
                .map_err(|err| CoreError::GameplayFailed {
                    system: "test.picky".to_owned(),
                    message: format!("{err:?}"),
                })?;
        if !parsed.ok {
            return Err(CoreError::GameplayFailed {
                system: "test.picky".to_owned(),
                message: "`ok` is false".to_owned(),
            });
        }
        Ok(())
    }

    fn step(&mut self, _ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        None
    }

    fn cancel(&mut self) {}
}

/// Records that `cancel` ran, so the test can prove the check cleans up after itself.
struct Cancelling {
    cancels: std::rc::Rc<std::cell::Cell<usize>>,
}

impl GameplaySystem for Cancelling {
    fn id(&self) -> SystemId {
        SystemId::from("test.cancelling")
    }

    fn begin(
        &mut self,
        _config: Option<&ron::Value>,
        _narrative: &NarrativeSnapshot,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    fn step(&mut self, _ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        None
    }

    fn cancel(&mut self) {
        self.cancels.set(self.cancels.get() + 1);
    }
}

fn handover(system: &str, config: Option<&str>) -> Exit {
    Exit::Gameplay {
        spec: GameplaySpec {
            system: SystemId::from(system),
            config: config.map(|text| ron::from_str(text).expect("valid RON literal")),
            outcomes: BTreeMap::new(),
        },
    }
}

fn story_of(nodes: Vec<(&str, Exit)>) -> Story {
    let nodes: BTreeMap<NodeId, Node> = nodes
        .into_iter()
        .map(|(id, exit)| {
            (
                NodeId::from(id),
                Node {
                    beats: Vec::new(),
                    exit,
                },
            )
        })
        .collect();
    Story::new("start", nodes)
}

#[test]
fn a_valid_config_produces_no_complaints() {
    let story = story_of(vec![("start", handover("test.picky", Some("(ok:true)")))]);
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Picky));

    assert_eq!(registry.check_configs(&story), Vec::<String>::new());
}

#[test]
fn a_config_the_system_rejects_is_reported_with_its_node() {
    let story = story_of(vec![
        ("start", handover("test.picky", Some("(ok:false)"))),
        ("good", Exit::End { ending: None }),
    ]);
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Picky));

    let found = registry.check_configs(&story);

    assert_eq!(found.len(), 1, "got {found:?}");
    assert!(found[0].contains("node `start`"), "got {}", found[0]);
    assert!(found[0].contains("test.picky"), "got {}", found[0]);
    assert!(found[0].contains("ok"), "got {}", found[0]);
}

#[test]
fn a_missing_config_counts_as_rejected() {
    // The system decides whether a config is optional; the engine must not assume either way.
    let story = story_of(vec![("start", handover("test.picky", None))]);
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Picky));

    let found = registry.check_configs(&story);

    assert_eq!(found.len(), 1, "got {found:?}");
    assert!(
        found[0].contains("required"),
        "the system's own complaint must reach the caller: {}",
        found[0]
    );
}

#[test]
fn every_bad_node_is_reported_not_just_the_first() {
    // One fault at a time is right for the editor, where the author fixes and re-checks. A
    // pre-release check that stops at the first bad scene wastes a round trip per scene.
    let story = story_of(vec![
        ("one", handover("test.picky", Some("(ok:false)"))),
        ("two", handover("test.picky", Some("(ok:false)"))),
        ("three", handover("test.picky", Some("(ok:true)"))),
    ]);
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Picky));

    let found = registry.check_configs(&story);

    assert_eq!(found.len(), 2, "got {found:?}");
    assert!(found[0].contains("node `one`"));
    assert!(found[1].contains("node `two`"));
}

#[test]
fn an_unregistered_system_is_not_reported_as_a_bad_config() {
    // A missing system is a different fault with a different fix, and the caller asks about it
    // separately. Reporting it twice under two names would send an author looking in two places.
    let story = story_of(vec![("start", handover("test.nobody", Some("(x:1)")))]);
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Picky));

    assert_eq!(registry.check_configs(&story), Vec::<String>::new());
}

#[test]
fn checking_leaves_no_residue_in_the_systems() {
    let story = story_of(vec![
        ("start", handover("test.cancelling", None)),
        ("second", handover("test.cancelling", None)),
    ]);
    let cancels = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Cancelling {
        cancels: cancels.clone(),
    }));

    registry.check_configs(&story);

    assert_eq!(
        cancels.get(),
        2,
        "every node checked must be cancelled, ready for the next real handover"
    );
}

#[test]
fn a_story_with_no_handovers_is_checked_in_no_time_and_says_nothing() {
    let story = story_of(vec![("start", Exit::End { ending: None })]);
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Picky));

    assert_eq!(registry.check_configs(&story), Vec::<String>::new());
}
