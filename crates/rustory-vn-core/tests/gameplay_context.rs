//! What a gameplay system can know about the story around it.
//!
//! The theme: a system whose rules depend on context must get that context from the engine,
//! not reconstruct it. Before `NarrativeSnapshot` the only way was a counter in `Stats` that
//! nothing kept in step with the story.

use std::collections::BTreeMap;

use rustory_vn_core::prelude::*;

/// A system that reports a different outcome depending on how many times its node was
/// entered. This is the pattern the snapshot exists for: a boss that is harder the second
/// time, a question that changes once you have heard the answer.
#[derive(Default)]
struct RoundAware {
    visits: usize,
    has_visited_secret: bool,
}

impl GameplaySystem for RoundAware {
    fn id(&self) -> SystemId {
        SystemId::from("test.round_aware")
    }

    fn begin(
        &mut self,
        _config: Option<&ron::Value>,
        narrative: &NarrativeSnapshot,
    ) -> Result<(), CoreError> {
        // `begin` sees the snapshot too: a system that varies its content by context has to
        // decide before the first frame, not after it.
        self.visits = narrative.visits("arena");
        self.has_visited_secret = narrative.has_visited("secret");
        Ok(())
    }

    fn step(&mut self, _ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        Some(OutcomeKey::from(if self.has_visited_secret {
            "informed"
        } else if self.visits >= 2 {
            "harder"
        } else {
            "first"
        }))
    }

    fn cancel(&mut self) {
        self.visits = 0;
        self.has_visited_secret = false;
    }
}

fn say(text: &str) -> Beat {
    Beat::Say {
        speaker: None,
        text: text.into(),
    }
}

fn gameplay() -> Exit {
    Exit::Gameplay {
        spec: GameplaySpec {
            system: SystemId::from("test.round_aware"),
            config: None,
            outcomes: BTreeMap::from([
                (OutcomeKey::from("first"), NodeId::from("after_first")),
                (OutcomeKey::from("harder"), NodeId::from("after_second")),
                (OutcomeKey::from("informed"), NodeId::from("after_secret")),
            ]),
        },
    }
}

fn ending() -> Node {
    Node {
        beats: Vec::new(),
        exit: Exit::End { ending: None },
    }
}

/// `arena` hands to the gameplay every time; its outcome routes back to `arena` until the
/// third pass, so the system sees a growing visit count.
fn looping_story() -> Story {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        NodeId::from("arena"),
        Node {
            beats: vec![say("Round begins.")],
            exit: gameplay(),
        },
    );
    // Every outcome loops back to the arena, which is what makes visits grow.
    if let Some(node) = nodes.get_mut(&NodeId::from("arena"))
        && let Exit::Gameplay { spec } = &mut node.exit
    {
        for target in spec.outcomes.values_mut() {
            *target = NodeId::from("arena");
        }
    }
    nodes.insert(NodeId::from("after_first"), ending());
    nodes.insert(NodeId::from("after_second"), ending());
    nodes.insert(NodeId::from("after_secret"), ending());
    Story::new("arena", nodes)
}

/// Advance out of the beats, then step the gameplay handover once.
fn run_handover(narrative: &mut Narrative, registry: &mut GameplayRegistry) {
    // The handover happens when the narrative runs out of beats; force it.
    while matches!(narrative.mode(), Mode::Revealing | Mode::AwaitingAdvance) {
        narrative.advance().expect("advance");
    }
    narrative.take_events();

    // The system decides, and the story moves to the node its outcome key names.
    narrative.step_gameplay(registry, 0, &[]).expect("steps");
}

#[test]
fn a_system_sees_how_many_times_its_node_was_entered() {
    let story = looping_story();
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(RoundAware::default()));

    let mut narrative = Narrative::new(story).expect("valid");

    // First handover: one visit.
    assert_eq!(narrative.snapshot().visits("arena"), 1);
    run_handover(&mut narrative, &mut registry);

    // The story loops back to `arena`, so the second handover sees two visits.
    assert_eq!(narrative.cursor(), &NodeId::from("arena"));
    assert_eq!(
        narrative.snapshot().visits("arena"),
        2,
        "a system asking 'is this the second time' must be told yes"
    );
}

#[test]
fn a_system_sees_whether_a_node_was_ever_visited() {
    // The other question, which a visit count cannot answer as cheaply: has the player already
    // been through the scene that explains the rule?
    let story = looping_story();
    let snapshot = Narrative::new(story).expect("valid").snapshot();

    assert!(snapshot.has_visited("arena"));
    assert!(!snapshot.has_visited("secret"));
    assert_eq!(snapshot.visits("secret"), 0);
}

#[test]
fn the_snapshot_reports_the_position_at_the_handover() {
    let story = looping_story();
    let narrative = Narrative::new(story).expect("valid");
    let snapshot = narrative.snapshot();

    assert_eq!(snapshot.node, NodeId::from("arena"));
    assert_eq!(snapshot.beat_count, 1, "the node has one beat");
    assert_eq!(snapshot.visited, [NodeId::from("arena")]);
    assert!(snapshot.choices.is_empty(), "no branch has been taken");
}

#[test]
fn choices_the_player_made_are_recorded_in_order() {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        NodeId::from("pick"),
        Node {
            beats: Vec::new(),
            exit: Exit::Choose {
                prompt: None,
                choices: vec![
                    Choice {
                        label: "first".into(),
                        to: NodeId::from("second_pick"),
                    },
                    Choice {
                        label: "other".into(),
                        to: NodeId::from("end"),
                    },
                ],
            },
        },
    );
    nodes.insert(
        NodeId::from("second_pick"),
        Node {
            beats: Vec::new(),
            exit: Exit::Choose {
                prompt: None,
                choices: vec![Choice {
                    label: "only".into(),
                    to: NodeId::from("end"),
                }],
            },
        },
    );
    nodes.insert(NodeId::from("end"), ending());

    let mut narrative = Narrative::new(Story::new("pick", nodes)).expect("valid");
    narrative.choose(0).expect("first branch");
    narrative.choose(0).expect("second branch");

    let snapshot = narrative.snapshot();
    assert_eq!(
        snapshot.choices,
        [
            ChoiceMade {
                node: NodeId::from("pick"),
                index: 0,
            },
            ChoiceMade {
                node: NodeId::from("second_pick"),
                index: 0,
            },
        ]
    );
}

#[test]
fn a_save_carries_the_history_a_resumed_system_reads() {
    // Without this, a system resumed from a save would think it was the first visit — the
    // exact silent wrongness the snapshot exists to prevent.
    let story = looping_story();
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(RoundAware::default()));

    let mut narrative = Narrative::new(story.clone()).expect("valid");
    run_handover(&mut narrative, &mut registry);

    // Now at the second visit; save from here.
    let save = narrative.save(&registry).expect("saves");
    assert_eq!(save.visited, [NodeId::from("arena"), NodeId::from("arena")]);

    let resumed = Narrative::from_save(story, &save, &mut registry).expect("resumes");
    assert_eq!(
        resumed.snapshot().visits("arena"),
        2,
        "the history survives the round trip"
    );
}

#[test]
fn a_save_written_before_history_existed_still_loads() {
    // The field is `#[serde(default)]` for this: an older save describes a story with no
    // recorded history, which is what it had. Refusing it would lose progress over a feature
    // the player never used.
    let story = looping_story();
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(RoundAware::default()));

    let narrative = Narrative::new(story.clone()).expect("valid");
    let save = narrative.save(&registry).expect("saves");
    let text = ron::to_string(&save).expect("serialises");

    // RON writes this as one line, so the fields are removed by pattern rather than by line.
    let without_history = text
        .replace(",visited:[\"arena\"]", "")
        .replace(",choices:[]", "");
    assert_ne!(
        without_history, text,
        "the test must actually remove the fields"
    );

    let old: SaveState = ron::from_str(&without_history).expect("an older save parses");
    assert!(old.visited.is_empty());
    assert!(old.choices.is_empty());
    assert!(Narrative::from_save(story, &old, &mut registry).is_ok());
}
