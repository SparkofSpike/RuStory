//! Pointer input: what a system receives, and what it can do with it.
//!
//! The theme: a system can already draw a board. This is the half that lets a player touch it.
//! The two properties that matter are that the coordinate is in the view's design box (so a
//! resized window cannot change the rules) and that a press outside everything is not an answer.

use std::collections::BTreeMap;

use rustory_vn_core::prelude::*;

/// Picks whichever door card contains the press, and reports `left` or `right`.
struct Doors;

impl GameplaySystem for Doors {
    fn id(&self) -> SystemId {
        SystemId::from("test.doors")
    }

    fn begin(
        &mut self,
        _config: Option<&ron::Value>,
        _narrative: &NarrativeSnapshot,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        for input in ctx.inputs {
            let GameplayInput::Pointer {
                x,
                y,
                phase: PointerPhase::Pressed,
            } = input
            else {
                continue;
            };
            if (240.0..600.0).contains(x) && (260.0..480.0).contains(y) {
                return Some(OutcomeKey::from("left"));
            }
            if (680.0..1040.0).contains(x) && (260.0..480.0).contains(y) {
                return Some(OutcomeKey::from("right"));
            }
            // A press on the background is not a choice. Returning an outcome here would let a
            // stray click end the interaction.
        }
        None
    }

    fn cancel(&mut self) {}
}

fn press(x: f32, y: f32) -> GameplayInput {
    GameplayInput::Pointer {
        x,
        y,
        phase: PointerPhase::Pressed,
    }
}

fn snapshot() -> NarrativeSnapshot {
    NarrativeSnapshot {
        node: NodeId::from("test"),
        beat_index: 0,
        beat_count: 0,
        visited: Vec::new(),
        choices: Vec::new(),
    }
}

fn run(inputs: &[GameplayInput]) -> Option<OutcomeKey> {
    let mut system = Doors;
    system.begin(None, &snapshot()).expect("begins");
    let position = snapshot();
    let mut stats = Stats::default();
    system.step(&mut GameplayContext {
        dt_ms: 16,
        inputs,
        stats: &mut stats,
        narrative: &position,
    })
}

#[test]
fn a_press_inside_a_card_picks_that_card() {
    assert_eq!(run(&[press(420.0, 370.0)]), Some(OutcomeKey::from("left")));
    assert_eq!(run(&[press(860.0, 370.0)]), Some(OutcomeKey::from("right")));
}

#[test]
fn a_press_on_the_background_picks_nothing() {
    // The whole point of two phases and a position: a click somewhere meaningless must be
    // ignored, not treated as "confirm whatever is highlighted".
    for (x, y) in [(10.0, 10.0), (640.0, 700.0), (420.0, 500.0)] {
        assert_eq!(run(&[press(x, y)]), None, "({x}, {y}) must not answer");
    }
}

#[test]
fn a_release_is_not_a_press() {
    // A drag sends a press and then a release; a system that treated both as answers would
    // answer twice, and the second one would be wherever the drag ended.
    let release = GameplayInput::Pointer {
        x: 420.0,
        y: 370.0,
        phase: PointerPhase::Released,
    };
    assert_eq!(run(&[release]), None);
}

#[test]
fn the_press_and_the_confirm_arrive_together_without_confusing_a_system() {
    // The app sends both, so a countdown that only knows `Confirm` keeps working and a board that
    // only knows `Pointer` is not answered twice.
    let both = [press(10.0, 10.0), GameplayInput::Confirm];
    assert_eq!(
        run(&both),
        None,
        "a press outside the cards plus a confirm is still not a choice"
    );
}

#[test]
fn a_pointer_input_survives_a_round_trip_through_the_wire_format() {
    // Input is not saved, but it *is* serialised across the wasm boundary. A guest that could not
    // parse what the host writes would see an empty input list and never answer.
    //
    // The host sends a slice, which RON writes as a parenthesised list; a hand-written guest looks
    // for `Pressed` and the two numbers, so the *shape* is the interface here as much as the
    // field names are.
    let inputs = vec![press(420.5, 370.25), GameplayInput::Confirm];
    let text = ron::ser::to_string(&inputs).expect("serialises");

    assert!(
        text.contains("Pointer"),
        "the variant name is what a guest greps for: {text}"
    );
    assert!(text.contains("Pressed"), "the phase is a word: {text}");
    assert!(
        text.contains("420.5") && text.contains("370.25"),
        "coordinates keep their precision: {text}"
    );

    let back: Vec<GameplayInput> = ron::from_str(&text).expect("parses");
    assert_eq!(back, inputs);
}

#[test]
fn a_pointer_story_runs_end_to_end() {
    // The same shape a real story uses, driven through the narrative rather than by calling a
    // system directly: the handover, the press, and the branch it selects.
    let mut nodes = BTreeMap::new();
    nodes.insert(
        NodeId::from("hall"),
        Node {
            beats: Vec::new(),
            exit: Exit::Gameplay {
                spec: GameplaySpec {
                    system: SystemId::from("test.doors"),
                    config: None,
                    outcomes: BTreeMap::from([
                        (OutcomeKey::from("left"), NodeId::from("left_end")),
                        (OutcomeKey::from("right"), NodeId::from("right_end")),
                    ]),
                },
            },
        },
    );
    for id in ["left_end", "right_end"] {
        nodes.insert(
            NodeId::from(id),
            Node {
                beats: Vec::new(),
                exit: Exit::End {
                    ending: Some(id.to_owned()),
                },
            },
        );
    }
    let story = Story::new("hall", nodes);

    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Doors));
    let mut narrative = Narrative::new(story).expect("valid");

    // Entering the node hands over immediately, since it has no beats.
    assert!(matches!(narrative.mode(), Mode::Suspended { .. }));
    narrative
        .step_gameplay(&mut registry, 16, &[press(860.0, 370.0)])
        .expect("steps");

    assert_eq!(
        narrative.cursor(),
        &NodeId::from("right_end"),
        "the press chose the right card, so the story took the branch it maps"
    );
}
