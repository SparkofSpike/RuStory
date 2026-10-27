//! Save and load behaviour.
//!
//! The theme of every test here is the same: a save that fails to restore must
//! fail *loudly*. A player who loses progress can start again; a player who
//! resumes in the wrong scene, or with someone else's counters, has no way to tell
//! whether the engine is broken.

use std::collections::BTreeMap;

use rustory_vn_core::prelude::*;

fn make_story(nodes: Vec<(&str, Node)>, start: &str) -> Story {
    Story::new(
        start,
        nodes
            .into_iter()
            .map(|(id, node)| (NodeId::from(id), node))
            .collect(),
    )
}

fn say(text: &str) -> Beat {
    Beat::Say {
        speaker: None,
        text: text.into(),
    }
}

fn goto(to: &str) -> Exit {
    Exit::Goto {
        to: NodeId::from(to),
    }
}

/// A story with a line, a branch and an ending.
fn branching_story() -> Story {
    make_story(
        vec![
            (
                "start",
                Node {
                    beats: vec![
                        Beat::SetBackground {
                            image: Some("room.png".into()),
                        },
                        Beat::ShowActor {
                            actor: ActorId::from("yuki"),
                            pose: "smile".into(),
                            slot: Slot::Left,
                        },
                        say("Where were you?"),
                    ],
                    exit: Exit::Choose {
                        prompt: Some("Answer".into()),
                        choices: vec![
                            Choice {
                                label: "Truth".into(),
                                to: NodeId::from("honest"),
                            },
                            Choice {
                                label: "Lie".into(),
                                to: NodeId::from("lie"),
                            },
                        ],
                    },
                },
            ),
            (
                "honest",
                Node {
                    beats: vec![say("Thank you.")],
                    exit: Exit::End {
                        ending: Some("honest".into()),
                    },
                },
            ),
            (
                "lie",
                Node {
                    beats: vec![say("I see.")],
                    exit: Exit::End {
                        ending: Some("lie".into()),
                    },
                },
            ),
        ],
        "start",
    )
}

#[test]
fn a_save_mid_line_resumes_at_the_same_line() {
    let story = branching_story();
    let mut narrative = Narrative::new(story.clone()).expect("valid");
    let registry = GameplayRegistry::new();

    narrative.tick(16, 5);
    let save = narrative.save(&registry).expect("saves");

    let resumed =
        Narrative::from_save(story, &save, &mut GameplayRegistry::new()).expect("restores");

    assert_eq!(resumed.cursor().as_str(), narrative.cursor().as_str());
    assert_eq!(resumed.mode(), narrative.mode());
    assert_eq!(
        resumed.line().map(|line| line.visible),
        narrative.line().map(|line| line.visible),
        "the reveal position must resume, not restart"
    );
}

#[test]
fn a_save_restores_the_stage_and_the_counters() {
    let story = branching_story();
    let mut narrative = Narrative::new(story.clone()).expect("valid");
    let registry = GameplayRegistry::new();

    // Walk to the end of the beats so a choice is pending, then take the first
    // branch after recording what the stage looked like.
    narrative.tick(1000, 0);
    narrative.advance().expect("advance");
    let save = narrative.save(&registry).expect("saves");
    assert_eq!(save.stage.background(), Some("room.png"));
    assert!(save.stage.actor(&ActorId::from("yuki")).is_some());

    let resumed =
        Narrative::from_save(story, &save, &mut GameplayRegistry::new()).expect("restores");

    assert_eq!(resumed.stage().background(), Some("room.png"));
    let actor = resumed
        .stage()
        .actor(&ActorId::from("yuki"))
        .expect("the sprite is still on stage");
    assert_eq!(actor.pose, "smile");
    assert_eq!(actor.slot, Slot::Left);
}

#[test]
fn a_pending_choice_survives_the_round_trip() {
    let story = branching_story();
    let mut narrative = Narrative::new(story.clone()).expect("valid");
    let registry = GameplayRegistry::new();

    narrative.tick(1000, 0);
    narrative.advance().expect("advance");
    let save = narrative.save(&registry).expect("saves");

    let mut resumed =
        Narrative::from_save(story, &save, &mut GameplayRegistry::new()).expect("restores");

    let Mode::AwaitingChoice { choices, prompt } = resumed.mode() else {
        panic!("expected a pending choice, got {:?}", resumed.mode());
    };
    assert_eq!(choices.len(), 2);
    assert_eq!(prompt.as_deref(), Some("Answer"));

    // And the choice still works after restoring.
    resumed.choose(1).expect("choose");
    resumed.tick(1000, 0);
    assert_eq!(resumed.line().map(|line| line.visible), Some("I see."));
}

#[test]
fn a_finished_story_saves_and_restores_as_finished() {
    let story = branching_story();
    let mut narrative = Narrative::new(story.clone()).expect("valid");
    let registry = GameplayRegistry::new();

    narrative.tick(1000, 0);
    narrative.advance().expect("advance");
    narrative.choose(0).expect("choose");
    narrative.tick(1000, 0);
    narrative.advance().expect("advance");

    let save = narrative.save(&registry).expect("saves");
    let resumed =
        Narrative::from_save(story, &save, &mut GameplayRegistry::new()).expect("restores");

    assert!(
        matches!(resumed.mode(), Mode::Finished { ending: Some(ending) } if ending == "honest"),
        "got {:?}",
        resumed.mode()
    );
}

#[test]
fn a_save_from_a_different_story_is_refused() {
    let original = branching_story();
    let narrative = Narrative::new(original).expect("valid");
    let save = narrative.save(&GameplayRegistry::new()).expect("saves");

    // A beat inserted before the cursor: the saved index now means a different line, so the
    // save must be refused rather than resumed somewhere else.
    let mut edited = branching_story();
    if let Some(node) = edited.nodes.get_mut(&NodeId::from("start")) {
        node.beats.insert(1, say("An inserted line."));
    }

    let err =
        Narrative::from_save(edited, &save, &mut GameplayRegistry::new()).expect_err("must refuse");
    assert!(
        matches!(err, CoreError::SaveStoryMismatch { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn rewriting_a_line_keeps_a_save_valid() {
    // The complement of the test above, and the reason the fingerprint covers shape and not
    // text: a save remembers a position, and fixing a typo does not move anything. Refusing
    // the save would punish an author for an edit that cannot misplace a player.
    let original = branching_story();
    let narrative = Narrative::new(original).expect("valid");
    let save = narrative.save(&GameplayRegistry::new()).expect("saves");

    let mut edited = branching_story();
    if let Some(node) = edited.nodes.get_mut(&NodeId::from("start")) {
        node.beats[2] = say("Where were you?  ");
    }

    let resumed = Narrative::from_save(edited, &save, &mut GameplayRegistry::new())
        .expect("a rewritten line is still the same position");
    assert_eq!(resumed.cursor(), &NodeId::from("start"));
}

#[test]
fn a_save_with_a_future_version_is_refused() {
    let story = branching_story();
    let narrative = Narrative::new(story.clone()).expect("valid");
    let mut save = narrative.save(&GameplayRegistry::new()).expect("saves");
    save.version = SAVE_VERSION + 1;

    let err =
        Narrative::from_save(story, &save, &mut GameplayRegistry::new()).expect_err("must refuse");
    assert!(
        matches!(err, CoreError::SaveVersionMismatch { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn a_save_pointing_at_a_missing_node_is_refused() {
    let story = branching_story();
    let narrative = Narrative::new(story.clone()).expect("valid");
    let mut save = narrative.save(&GameplayRegistry::new()).expect("saves");
    save.cursor = NodeId::from("does_not_exist");

    let err =
        Narrative::from_save(story, &save, &mut GameplayRegistry::new()).expect_err("must refuse");
    assert!(
        matches!(err, CoreError::SaveNodeMissing { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn a_beat_index_past_the_node_is_refused() {
    let story = branching_story();
    let narrative = Narrative::new(story.clone()).expect("valid");
    let mut save = narrative.save(&GameplayRegistry::new()).expect("saves");
    save.beat_index = 99;

    let err =
        Narrative::from_save(story, &save, &mut GameplayRegistry::new()).expect_err("must refuse");
    assert!(
        matches!(err, CoreError::SaveBeatOutOfRange { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn a_mode_that_does_not_fit_the_node_exit_is_refused() {
    let story = branching_story();
    let narrative = Narrative::new(story.clone()).expect("valid");
    let mut save = narrative.save(&GameplayRegistry::new()).expect("saves");

    // Claim the story is waiting for a choice in a node whose exit is not one.
    let mut mut_story = story;
    if let Some(node) = mut_story.nodes.get_mut(&NodeId::from("start")) {
        node.exit = goto("honest");
        node.beats.clear();
    }
    save.beat_index = 0;
    save.mode = Mode::AwaitingChoice {
        prompt: None,
        choices: vec![],
    };
    // The checksum now differs too, so rebuild it to isolate the mode check.
    save.story_checksum = mut_story.checksum();

    let err = Narrative::from_save(mut_story, &save, &mut GameplayRegistry::new())
        .expect_err("must refuse");
    assert!(
        matches!(err, CoreError::SaveBeatOutOfRange { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn a_reveal_count_beyond_the_line_is_clamped_rather_than_rejected() {
    let story = branching_story();
    let narrative = Narrative::new(story.clone()).expect("valid");
    let mut save = narrative.save(&GameplayRegistry::new()).expect("saves");
    save.revealed = 9_999;

    let resumed = Narrative::from_save(story, &save, &mut GameplayRegistry::new())
        .expect("clamping is safer than refusing here");
    assert_eq!(
        resumed.line().map(|line| line.visible),
        Some("Where were you?"),
        "the visible portion must stop at the end of the line"
    );
}

#[test]
fn the_same_state_serialises_to_the_same_text() {
    let story = branching_story();
    let narrative = Narrative::new(story).expect("valid");
    let registry = GameplayRegistry::new();

    let a = narrative.save(&registry).expect("saves");
    let b = narrative.save(&registry).expect("saves");

    assert_eq!(
        a.to_ron().expect("serialises"),
        b.to_ron().expect("serialises"),
        "saves must be diffable; unstable ordering would make them noise"
    );
}

#[test]
fn a_story_checksum_changes_when_the_script_changes() {
    let a = branching_story().checksum();
    let mut edited = branching_story();
    if let Some(node) = edited.nodes.get_mut(&NodeId::from("start")) {
        node.beats.push(say("One more line."));
    }
    assert_ne!(
        a,
        edited.checksum(),
        "the checksum must be content-sensitive"
    );

    let same = branching_story().checksum();
    assert_eq!(a, same, "the same script must hash identically every time");
}

// --- gameplay state through a save ---------------------------------------

/// A system whose state is a growing list, to prove saves carry more than scalars.
struct Recorder {
    config: Option<ron::Value>,
    seen: Vec<u32>,
}

impl GameplaySystem for Recorder {
    fn id(&self) -> SystemId {
        SystemId::from("test.recorder")
    }

    fn begin(
        &mut self,
        config: Option<&ron::Value>,
        _narrative: &NarrativeSnapshot,
    ) -> Result<(), CoreError> {
        self.config = config.cloned();
        self.seen.clear();
        Ok(())
    }

    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        for input in ctx.inputs {
            if let GameplayInput::Select { index } = input {
                self.seen.push(*index as u32);
            }
        }
        if self.seen.len() >= 2 {
            return Some(OutcomeKey::from("done"));
        }
        None
    }

    fn cancel(&mut self) {
        self.config = None;
        self.seen.clear();
    }

    fn save(&self) -> Result<Option<ron::Value>, CoreError> {
        encode_state(&self.seen)
            .map(Some)
            .map_err(|message| CoreError::GameplaySaveFailed {
                system: "test.recorder".into(),
                message,
            })
    }

    fn restore(&mut self, state: Option<&ron::Value>) -> Result<(), CoreError> {
        let Some(value) = state else {
            return Ok(());
        };
        self.seen = decode_state(value).map_err(|message| CoreError::GameplayRestoreFailed {
            system: "test.recorder".into(),
            message,
        })?;
        Ok(())
    }
}

fn gameplay_story() -> Story {
    make_story(
        vec![
            (
                "play",
                Node {
                    beats: vec![say("go")],
                    exit: Exit::Gameplay {
                        spec: GameplaySpec {
                            system: SystemId::from("test.recorder"),
                            config: Some(ron::from_str("1").expect("literal")),
                            outcomes: [(OutcomeKey::from("done"), NodeId::from("after"))]
                                .into_iter()
                                .collect(),
                        },
                    },
                },
            ),
            (
                "after",
                Node {
                    beats: vec![say("after")],
                    exit: Exit::End { ending: None },
                },
            ),
        ],
        "play",
    )
}

#[test]
fn gameplay_state_survives_a_save_and_resume() {
    let story = gameplay_story();
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Recorder {
        config: None,
        seen: Vec::new(),
    }));

    let mut narrative = Narrative::new(story.clone()).expect("valid");
    narrative.tick(1000, 0);
    narrative.advance().expect("advance");
    assert!(matches!(narrative.mode(), Mode::Suspended { .. }));

    // Take one of the two steps needed, then save mid-gameplay.
    narrative
        .step_gameplay(&mut registry, 16, &[GameplayInput::Select { index: 7 }])
        .expect("step");
    assert!(matches!(narrative.mode(), Mode::Suspended { .. }));

    let save = narrative.save(&registry).expect("saves");
    let gameplay = save.gameplay.as_ref().expect("state must be recorded");
    assert_eq!(gameplay.system.as_str(), "test.recorder");
    assert!(gameplay.state.is_some(), "the list must be persisted");

    // Resume with a fresh registry, as a new process would.
    let mut resumed_registry = GameplayRegistry::new();
    resumed_registry.register(Box::new(Recorder {
        config: None,
        seen: Vec::new(),
    }));
    let mut resumed = Narrative::from_save(story, &save, &mut resumed_registry).expect("restores");
    assert!(matches!(resumed.mode(), Mode::Suspended { .. }));

    // The first answer must still count, so one more step finishes it.
    resumed
        .step_gameplay(
            &mut resumed_registry,
            16,
            &[GameplayInput::Select { index: 9 }],
        )
        .expect("step");
    assert_eq!(
        resumed.cursor().as_str(),
        "after",
        "the restored list should have been one short of the goal"
    );
}

#[test]
fn a_suspended_save_without_gameplay_state_is_refused() {
    let story = gameplay_story();
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(Recorder {
        config: None,
        seen: Vec::new(),
    }));

    let mut narrative = Narrative::new(story.clone()).expect("valid");
    narrative.tick(1000, 0);
    narrative.advance().expect("advance");

    let mut save = narrative.save(&registry).expect("saves");
    save.gameplay = None;

    let mut fresh = GameplayRegistry::new();
    fresh.register(Box::new(Recorder {
        config: None,
        seen: Vec::new(),
    }));
    let err = Narrative::from_save(story, &save, &mut fresh).expect_err("must refuse");
    assert!(
        matches!(err, CoreError::GameplayRestoreFailed { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn a_system_that_cannot_restore_rejects_state_rather_than_dropping_it() {
    /// Implements `save` but deliberately not `restore`.
    struct SavesOnly;

    impl GameplaySystem for SavesOnly {
        fn id(&self) -> SystemId {
            SystemId::from("test.saves_only")
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
        fn cancel(&mut self) {}
        fn save(&self) -> Result<Option<ron::Value>, CoreError> {
            Ok(Some(ron::from_str("(secret: 42)").expect("literal")))
        }
    }

    let story = make_story(
        vec![(
            "play",
            Node {
                beats: vec![],
                exit: Exit::Gameplay {
                    spec: GameplaySpec {
                        system: SystemId::from("test.saves_only"),
                        config: None,
                        outcomes: BTreeMap::new(),
                    },
                },
            },
        )],
        "play",
    );

    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(SavesOnly));
    let narrative = Narrative::new(story.clone()).expect("valid");
    let save = narrative.save(&registry).expect("saves");

    let mut fresh = GameplayRegistry::new();
    fresh.register(Box::new(SavesOnly));
    let err = Narrative::from_save(story, &save, &mut fresh).expect_err("must refuse");
    assert!(
        matches!(err, CoreError::GameplayRestoreFailed { .. }),
        "silently discarding state would resume the scene with invisible progress lost: {err}"
    );
}

// --- audio in saves -----------------------------------------------------

#[test]
fn the_playing_track_survives_a_save_and_load() {
    // Resuming in the middle of a scene must bring the music back. A host that only
    // watched events would have no idea what was playing when the file was written, which
    // is why the narrative carries the track rather than leaving it to the player.
    let story = make_story(
        vec![(
            "only",
            Node {
                beats: vec![
                    Beat::Music {
                        track: Some("classroom".into()),
                        fade_ms: None,
                    },
                    say("a line"),
                ],
                exit: Exit::End { ending: None },
            },
        )],
        "only",
    );

    let mut registry = GameplayRegistry::new();
    let narrative = Narrative::new(story.clone()).expect("valid");
    assert_eq!(narrative.music(), Some("classroom"));

    let save = narrative.save(&registry).expect("saves");
    assert_eq!(save.music.as_deref(), Some("classroom"));

    let resumed = Narrative::from_save(story, &save, &mut registry).expect("resumes");
    assert_eq!(
        resumed.music(),
        Some("classroom"),
        "a resumed scene plays the track it was saved with"
    );
}

#[test]
fn a_save_written_before_audio_existed_still_loads() {
    // The field is `#[serde(default)]` for exactly this: an older save describes a silent
    // scene, which is what it was. Refusing it would lose a player's progress over a
    // feature they never used.
    let story = make_story(
        vec![(
            "only",
            Node {
                beats: vec![say("a line")],
                exit: Exit::End { ending: None },
            },
        )],
        "only",
    );

    let mut registry = GameplayRegistry::new();
    let narrative = Narrative::new(story.clone()).expect("valid");
    let save = narrative.save(&registry).expect("saves");
    let text = save.to_ron().expect("serialises");

    // Remove the line the new build writes; what is left is a pre-audio save.
    let without_music: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("music:"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(without_music, text, "the test must actually remove a line");

    let old = SaveState::from_ron(&without_music).expect("an older save parses");
    assert_eq!(old.music, None);
    assert!(Narrative::from_save(story, &old, &mut registry).is_ok());
}
