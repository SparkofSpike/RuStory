//! Behavioural tests for the narrative state machine.
//!
//! These use only the public API, so they double as a check that the crate is
//! actually usable from outside — a doctest-shaped contract test.

use rustory_core::prelude::*;

/// Instant text: `chars_per_sec == 0` reveals the whole line on the first tick.
const INSTANT: u32 = 0;

fn make_story(nodes: Vec<(&str, Node)>, start: &str) -> Story {
    Story {
        meta: StoryMeta::default(),
        start: NodeId::from(start),
        nodes: nodes
            .into_iter()
            .map(|(id, node)| (NodeId::from(id), node))
            .collect(),
    }
}

fn say(text: &str) -> Beat {
    Beat::Say {
        speaker: None,
        text: text.to_owned(),
    }
}

fn end() -> Exit {
    Exit::End { ending: None }
}

fn goto(to: &str) -> Exit {
    Exit::Goto {
        to: NodeId::from(to),
    }
}

/// Drain events and keep the ones matching a predicate.
fn drain_matching(
    narrative: &mut Narrative,
    predicate: impl Fn(&NarrativeEvent) -> bool,
) -> Vec<NarrativeEvent> {
    narrative
        .take_events()
        .into_iter()
        .filter(predicate)
        .collect()
}

#[test]
fn plays_a_linear_story_to_the_end() {
    let mut narrative = Narrative::new(make_story(
        vec![(
            "only",
            Node {
                beats: vec![say("first"), say("second")],
                exit: end(),
            },
        )],
        "only",
    ))
    .expect("story is valid");

    assert!(matches!(narrative.mode(), Mode::Revealing));

    for expected in ["first", "second"] {
        narrative.tick(1000, INSTANT);
        assert!(matches!(narrative.mode(), Mode::AwaitingAdvance));
        assert_eq!(narrative.line().map(|l| l.visible), Some(expected));
        narrative.advance().expect("advance");
    }

    assert!(matches!(narrative.mode(), Mode::Finished { .. }));
}

#[test]
fn typewriter_reveals_on_grapheme_boundaries() {
    // A family emoji is one grapheme but several chars. Slicing by char would
    // produce half an emoji, which renders as a replacement glyph.
    let mut narrative = Narrative::new(make_story(
        vec![(
            "only",
            Node {
                beats: vec![say("a👨‍👩‍👧b")],
                exit: end(),
            },
        )],
        "only",
    ))
    .expect("story is valid");

    narrative.tick(10, 2); // ~0.02 graphemes of progress
    assert_eq!(narrative.line().map(|l| l.visible), Some(""));

    narrative.tick(1000, 3); // 3 graphemes
    assert_eq!(narrative.line().map(|l| l.visible), Some("a👨‍👩‍👧b"));

    narrative.tick(1000, 4);
    assert!(matches!(narrative.mode(), Mode::AwaitingAdvance));
    assert_eq!(
        narrative.line().map(|l| l.visible),
        Some("a👨‍👩‍👧b"),
        "reveal must clamp to the full line, never overshoot"
    );
}

#[test]
fn advancing_while_revealing_completes_the_line_first() {
    let mut narrative = Narrative::new(make_story(
        vec![(
            "only",
            Node {
                beats: vec![say("slowly spoken line")],
                exit: end(),
            },
        )],
        "only",
    ))
    .expect("story is valid");

    narrative.tick(16, 5);
    assert!(matches!(narrative.mode(), Mode::Revealing));

    narrative.advance().expect("advance");
    assert!(
        matches!(narrative.mode(), Mode::AwaitingAdvance),
        "first click completes the line, it does not skip it"
    );
    assert_eq!(
        narrative.line().map(|l| l.visible),
        Some("slowly spoken line")
    );

    narrative.advance().expect("advance");
    assert!(matches!(narrative.mode(), Mode::Finished { .. }));
}

#[test]
fn stage_effects_apply_before_the_player_reaches_the_line() {
    let mut narrative = Narrative::new(make_story(
        vec![(
            "only",
            Node {
                beats: vec![
                    Beat::SetBackground {
                        image: Some("school.png".into()),
                    },
                    Beat::ShowActor {
                        actor: ActorId::from("yuki"),
                        pose: "smile".into(),
                        slot: Slot::Center,
                    },
                    say("hello"),
                ],
                exit: end(),
            },
        )],
        "only",
    ))
    .expect("story is valid");

    // No tick yet: the line is still unread, but the stage is already set. This
    // is what lets the renderer fade a sprite in while text types out.
    assert_eq!(narrative.stage().background(), Some("school.png"));
    let actor = narrative.stage().actor(&ActorId::from("yuki")).unwrap();
    assert_eq!(actor.pose, "smile");
    assert_eq!(actor.slot, Slot::Center);

    let shown = drain_matching(&mut narrative, |e| {
        matches!(e, NarrativeEvent::ActorShown { .. })
    });
    assert_eq!(shown.len(), 1);
}

#[test]
fn hiding_an_absent_actor_is_a_no_op() {
    let narrative = Narrative::new(make_story(
        vec![(
            "only",
            Node {
                beats: vec![
                    Beat::HideActor {
                        actor: ActorId::from("ghost"),
                    },
                    say("done"),
                ],
                exit: end(),
            },
        )],
        "only",
    ))
    .expect("story is valid");

    assert!(narrative.stage().actors().is_empty());
    assert!(matches!(narrative.mode(), Mode::Revealing));
}

#[test]
fn choices_branch_to_the_selected_node() {
    let mut narrative = Narrative::new(make_story(
        vec![
            (
                "ask",
                Node {
                    beats: vec![say("which way?")],
                    exit: Exit::Choose {
                        prompt: Some("choose".into()),
                        choices: vec![
                            Choice {
                                label: "left".into(),
                                to: NodeId::from("left"),
                            },
                            Choice {
                                label: "right".into(),
                                to: NodeId::from("right"),
                            },
                        ],
                    },
                },
            ),
            (
                "left",
                Node {
                    beats: vec![say("you went left")],
                    exit: end(),
                },
            ),
            (
                "right",
                Node {
                    beats: vec![say("you went right")],
                    exit: end(),
                },
            ),
        ],
        "ask",
    ))
    .expect("story is valid");

    narrative.tick(1000, INSTANT);
    narrative.advance().expect("advance");

    let Mode::AwaitingChoice { choices, prompt } = narrative.mode() else {
        panic!("expected a choice, got {:?}", narrative.mode());
    };
    assert_eq!(choices.len(), 2);
    assert_eq!(prompt.as_deref(), Some("choose"));

    narrative.choose(1).expect("choose");
    assert_eq!(narrative.cursor().as_str(), "right");
    narrative.tick(1000, INSTANT);
    assert_eq!(narrative.line().map(|l| l.visible), Some("you went right"));
}

#[test]
fn choosing_out_of_range_reports_the_real_count() {
    let mut narrative = Narrative::new(make_story(
        vec![(
            "ask",
            Node {
                beats: vec![],
                exit: Exit::Choose {
                    prompt: None,
                    choices: vec![Choice {
                        label: "only".into(),
                        to: NodeId::from("ask"),
                    }],
                },
            },
        )],
        "ask",
    ))
    .expect("story is valid");

    let err = narrative.choose(3).expect_err("must reject");
    assert!(
        matches!(
            err,
            CoreError::ChoiceOutOfRange {
                index: 3,
                count: 1,
                ..
            }
        ),
        "unexpected error: {err}"
    );
}

#[test]
fn choosing_when_not_asked_is_an_error_not_a_silent_no_op() {
    let mut narrative = Narrative::new(make_story(
        vec![(
            "only",
            Node {
                beats: vec![say("no choice here")],
                exit: end(),
            },
        )],
        "only",
    ))
    .expect("story is valid");

    assert!(matches!(
        narrative.choose(0),
        Err(CoreError::NotAwaitingChoice { .. })
    ));
}

#[test]
fn validation_rejects_a_dangling_goto() {
    let err = Narrative::new(make_story(
        vec![(
            "only",
            Node {
                beats: vec![],
                exit: goto("nowhere"),
            },
        )],
        "only",
    ))
    .expect_err("must reject");

    assert!(
        matches!(err, CoreError::DanglingNodeRef { ref target, .. } if target == "nowhere"),
        "unexpected error: {err}"
    );
}

#[test]
fn validation_rejects_a_missing_start_node() {
    let err = Narrative::new(make_story(
        vec![(
            "present",
            Node {
                beats: vec![],
                exit: end(),
            },
        )],
        "absent",
    ))
    .expect_err("must reject");

    assert!(
        matches!(err, CoreError::MissingStartNode(ref node) if node == "absent"),
        "unexpected error: {err}"
    );
}

#[test]
fn a_goto_cycle_without_dialogue_is_stopped() {
    // Two empty nodes pointing at each other. Without a bound this call never
    // returns and the window freezes with no explanation.
    let err = Narrative::new(make_story(
        vec![
            (
                "a",
                Node {
                    beats: vec![],
                    exit: goto("b"),
                },
            ),
            (
                "b",
                Node {
                    beats: vec![],
                    exit: goto("a"),
                },
            ),
        ],
        "a",
    ))
    .expect_err("must reject");

    assert!(
        matches!(err, CoreError::EndlessFlow { .. }),
        "unexpected error: {err}"
    );
}

// --- gameplay handover ---------------------------------------------------

/// A minimal system for testing: reports `done` after a set number of ticks.
struct Countdown {
    ticks_left: u32,
    began_with: Option<u32>,
}

impl Countdown {
    fn new() -> Self {
        Self {
            ticks_left: 0,
            began_with: None,
        }
    }
}

impl GameplaySystem for Countdown {
    fn id(&self) -> SystemId {
        SystemId::from("test.countdown")
    }

    fn begin(&mut self, config: Option<&ron::Value>) -> Result<(), CoreError> {
        let Some(value) = config else {
            self.ticks_left = 1;
            return Ok(());
        };
        self.ticks_left =
            value
                .clone()
                .into_rust::<u32>()
                .map_err(|_| CoreError::GameplayFailed {
                    system: self.id().to_string(),
                    message: "config must be an integer".into(),
                })?;
        self.began_with = Some(self.ticks_left);
        Ok(())
    }

    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        self.ticks_left = self.ticks_left.saturating_sub(1);
        if self.ticks_left == 0 {
            ctx.stats.add("rounds", 1);
            Some(OutcomeKey::from("done"))
        } else {
            None
        }
    }

    fn cancel(&mut self) {
        self.ticks_left = 0;
        self.began_with = None;
    }
}

fn gameplay_story() -> Story {
    make_story(
        vec![
            (
                "debate",
                Node {
                    beats: vec![say("make your case")],
                    exit: Exit::Gameplay {
                        spec: GameplaySpec {
                            system: SystemId::from("test.countdown"),
                            config: Some(
                                ron::from_str::<ron::Value>("2").expect("integer literal parses"),
                            ),
                            outcomes: [
                                (OutcomeKey::from("done"), NodeId::from("won")),
                                (OutcomeKey::from("fail"), NodeId::from("lost")),
                            ]
                            .into_iter()
                            .collect(),
                        },
                    },
                },
            ),
            (
                "won",
                Node {
                    beats: vec![say("you won")],
                    exit: end(),
                },
            ),
            (
                "lost",
                Node {
                    beats: vec![say("you lost")],
                    exit: end(),
                },
            ),
        ],
        "debate",
    )
}

#[test]
fn gameplay_takes_over_then_returns_control() {
    let mut narrative = Narrative::new(gameplay_story()).expect("story is valid");
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Countdown::new()));

    narrative.tick(1000, INSTANT);
    narrative.advance().expect("advance");

    assert!(
        matches!(narrative.mode(), Mode::Suspended { .. }),
        "expected the node to hand over, got {:?}",
        narrative.mode()
    );

    // Two ticks of the system, with the narrative forwarding each one.
    narrative
        .step_gameplay(&mut registry, 16, &[])
        .expect("first step");
    assert!(matches!(narrative.mode(), Mode::Suspended { .. }));

    narrative
        .step_gameplay(&mut registry, 16, &[])
        .expect("second step");

    assert_eq!(narrative.cursor().as_str(), "won");
    assert_eq!(narrative.stats().get("rounds"), 1);
    narrative.tick(1000, INSTANT);
    assert_eq!(narrative.line().map(|l| l.visible), Some("you won"));
}

#[test]
fn an_outcome_the_script_does_not_map_is_an_error() {
    let mut story = gameplay_story();
    // Drop the outcome the system actually reports.
    if let Some(node) = story.nodes.get_mut(&NodeId::from("debate"))
        && let Exit::Gameplay { spec } = &mut node.exit
    {
        spec.outcomes.remove(&OutcomeKey::from("done"));
    }

    let mut narrative = Narrative::new(story).expect("story is still structurally valid");
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Countdown::new()));

    narrative.tick(1000, INSTANT);
    narrative.advance().expect("advance");
    narrative
        .step_gameplay(&mut registry, 16, &[])
        .expect("first step");

    let err = narrative
        .step_gameplay(&mut registry, 16, &[])
        .expect_err("unmapped outcome must surface");
    assert!(
        matches!(err, CoreError::UnmappedOutcome { ref outcome, .. } if outcome == "done"),
        "unexpected error: {err}"
    );
}

#[test]
fn a_missing_gameplay_system_is_reported_not_ignored() {
    let mut narrative = Narrative::new(gameplay_story()).expect("story is valid");
    let mut empty_registry = GameplayRegistry::new();

    narrative.tick(1000, INSTANT);
    narrative.advance().expect("advance");

    let err = narrative
        .step_gameplay(&mut empty_registry, 16, &[])
        .expect_err("must report");
    assert!(
        matches!(err, CoreError::UnknownGameplaySystem { ref system, .. } if system == "test.countdown"),
        "unexpected error: {err}"
    );
}

#[test]
fn stepping_gameplay_while_not_suspended_does_nothing() {
    let mut narrative = Narrative::new(make_story(
        vec![(
            "only",
            Node {
                beats: vec![say("merely talking")],
                exit: end(),
            },
        )],
        "only",
    ))
    .expect("story is valid");
    let mut registry = GameplayRegistry::new();

    narrative
        .step_gameplay(&mut registry, 16, &[])
        .expect("no-op");
    assert!(matches!(narrative.mode(), Mode::Revealing));
}

#[test]
fn a_long_run_of_immediate_beats_is_not_mistaken_for_a_cycle() {
    // The traversal bound exists to catch goto cycles, not to cap scene length.
    // A scene that sets a lot of stage state before its first line is legitimate
    // and must still play.
    let mut beats: Vec<Beat> = (0..900)
        .map(|index| Beat::ShowActor {
            actor: ActorId::from(format!("actor{index}")),
            pose: "idle".into(),
            slot: Slot::Center,
        })
        .collect();
    beats.push(say("finally a line"));

    let mut narrative = Narrative::new(make_story(
        vec![("only", Node { beats, exit: end() })],
        "only",
    ))
    .expect("900 immediate beats is a long scene, not a loop");

    assert_eq!(narrative.stage().actors().len(), 900);
    narrative.tick(1000, INSTANT);
    assert_eq!(narrative.line().map(|l| l.visible), Some("finally a line"));
}

#[test]
fn a_single_node_cannot_exceed_the_traversal_bound() {
    // Past the bound the machine reports the problem instead of looping. Two
    // thousand stage commands in one node is a broken script, not a scene.
    let beats: Vec<Beat> = (0..2000)
        .map(|index| Beat::ShowActor {
            actor: ActorId::from(format!("actor{index}")),
            pose: "idle".into(),
            slot: Slot::Center,
        })
        .collect();

    let err = Narrative::new(make_story(
        vec![("only", Node { beats, exit: end() })],
        "only",
    ))
    .expect_err("must report, not hang");

    assert!(
        matches!(err, CoreError::EndlessFlow { .. }),
        "unexpected error: {err}"
    );
}
