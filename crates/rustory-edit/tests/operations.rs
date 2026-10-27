//! Editing operations.
//!
//! Two themes run through this file.
//!
//! **Nothing half-happens.** Every failure test asserts the document is exactly what
//! it was, because that is the property an agent depends on.
//!
//! **Refusals explain themselves.** Deleting a referenced node must say which
//! references block it; an error that just says "invalid" forces the caller to
//! reverse-engineer the cause.

use rustory_core::{Beat, Choice, Exit, Node, NodeId, Slot, Story, StoryMeta};
use rustory_edit::prelude::*;
use rustory_edit::{Document, Editor};

fn say(text: &str) -> Beat {
    Beat::Say {
        speaker: None,
        text: text.to_owned(),
    }
}

fn story_with(nodes: Vec<(&str, Node)>, start: &str) -> Story {
    Story {
        meta: StoryMeta::default(),
        start: NodeId::from(start),
        nodes: nodes
            .into_iter()
            .map(|(id, node)| (NodeId::from(id), node))
            .collect(),
    }
}

fn editor_of(story: Story) -> Editor {
    Editor::new(Document::new(story, "test.ron"))
}

/// A two-node story: `a` has one line and goes to `b`; `b` ends.
fn linked() -> Editor {
    editor_of(story_with(
        vec![
            (
                "a",
                Node {
                    beats: vec![say("first")],
                    exit: Exit::Goto {
                        to: NodeId::from("b"),
                    },
                },
            ),
            (
                "b",
                Node {
                    beats: vec![say("second")],
                    exit: Exit::End { ending: None },
                },
            ),
        ],
        "a",
    ))
}

#[test]
fn creating_a_node_adds_an_empty_one_that_ends() {
    let mut editor = linked();
    let before = editor.story().nodes.len();

    editor
        .apply(&EditOp::CreateNode {
            id: NodeId::from("c"),
            at: None,
        })
        .expect("creates");

    assert_eq!(editor.story().nodes.len(), before + 1);
    let node = &editor.story().nodes[&NodeId::from("c")];
    assert!(node.beats.is_empty());
    assert!(matches!(node.exit, Exit::End { .. }));
}

#[test]
fn creating_a_duplicate_id_is_refused() {
    let mut editor = linked();
    let snapshot = editor.story().clone();

    let err = editor
        .apply(&EditOp::CreateNode {
            id: NodeId::from("a"),
            at: None,
        })
        .expect_err("must refuse");

    assert!(matches!(err, EditError::NodeExists(ref id) if id.as_str() == "a"));
    assert_eq!(editor.story(), &snapshot, "a refused edit changes nothing");
}

#[test]
fn an_empty_node_id_is_refused() {
    let mut editor = linked();
    let err = editor
        .apply(&EditOp::CreateNode {
            id: NodeId::from("   "),
            at: None,
        })
        .expect_err("must refuse");
    assert!(matches!(err, EditError::EmptyNodeId(_)));
}

#[test]
fn deleting_an_unreferenced_node_works() {
    let mut editor = linked();
    editor
        .apply(&EditOp::CreateNode {
            id: NodeId::from("spare"),
            at: None,
        })
        .expect("creates");

    editor
        .apply(&EditOp::DeleteNode {
            id: NodeId::from("spare"),
        })
        .expect("deletes");

    assert!(!editor.story().nodes.contains_key(&NodeId::from("spare")));
}

#[test]
fn deleting_a_referenced_node_is_refused_and_says_what_blocks_it() {
    let mut editor = linked();
    let snapshot = editor.story().clone();

    let err = editor
        .apply(&EditOp::DeleteNode {
            id: NodeId::from("b"),
        })
        .expect_err("must refuse");

    let message = err.to_string();
    assert!(
        matches!(err, EditError::NodeInUse { .. }),
        "unexpected error: {message}"
    );
    assert!(
        message.contains('a'),
        "the error must name the referring node: {message}"
    );
    assert_eq!(editor.story(), &snapshot, "a refused edit changes nothing");
}

#[test]
fn deleting_the_start_node_is_refused() {
    let mut editor = linked();
    let err = editor
        .apply(&EditOp::DeleteNode {
            id: NodeId::from("a"),
        })
        .expect_err("must refuse");
    assert!(matches!(err, EditError::CannotDeleteStart { .. }));
}

#[test]
fn deleting_the_only_node_is_refused() {
    let mut editor = editor_of(story_with(
        vec![(
            "solo",
            Node {
                beats: vec![],
                exit: Exit::End { ending: None },
            },
        )],
        "solo",
    ));

    let err = editor
        .apply(&EditOp::DeleteNode {
            id: NodeId::from("solo"),
        })
        .expect_err("must refuse");
    assert!(matches!(err, EditError::CannotDeleteLast { .. }));
}

#[test]
fn renaming_rewrites_every_kind_of_reference() {
    // The interesting operation: doing this by hand in RON means finding four
    // different kinds of pointer.
    let mut editor = editor_of(story_with(
        vec![
            (
                "a",
                Node {
                    beats: vec![],
                    exit: Exit::Goto {
                        to: NodeId::from("target"),
                    },
                },
            ),
            (
                "b",
                Node {
                    beats: vec![],
                    exit: Exit::Choose {
                        prompt: None,
                        choices: vec![Choice {
                            label: "go".into(),
                            to: NodeId::from("target"),
                        }],
                    },
                },
            ),
            (
                "c",
                Node {
                    beats: vec![],
                    exit: Exit::Gameplay {
                        spec: GameplaySpec {
                            system: SystemId::from("t"),
                            config: None,
                            outcomes: [(OutcomeKey::from("win"), NodeId::from("target"))]
                                .into_iter()
                                .collect(),
                        },
                    },
                },
            ),
            (
                "target",
                Node {
                    beats: vec![say("here")],
                    exit: Exit::End { ending: None },
                },
            ),
        ],
        "target",
    ));

    editor
        .apply(&EditOp::RenameNode {
            from: NodeId::from("target"),
            to: NodeId::from("renamed"),
        })
        .expect("renames");

    let story = editor.story();
    assert_eq!(story.start.as_str(), "renamed", "start node follows");

    let a = &story.nodes[&NodeId::from("a")];
    assert!(
        matches!(&a.exit, Exit::Goto { to } if to.as_str() == "renamed"),
        "goto follows"
    );

    let b = &story.nodes[&NodeId::from("b")];
    let Exit::Choose { choices, .. } = &b.exit else {
        panic!("b should still offer choices");
    };
    assert_eq!(choices[0].to.as_str(), "renamed", "choice follows");

    let c = &story.nodes[&NodeId::from("c")];
    let Exit::Gameplay { spec } = &c.exit else {
        panic!("c should still be gameplay");
    };
    assert_eq!(
        spec.outcomes[&OutcomeKey::from("win")].as_str(),
        "renamed",
        "gameplay outcome follows"
    );

    assert!(!story.nodes.contains_key(&NodeId::from("target")));
    assert!(story.nodes.contains_key(&NodeId::from("renamed")));
}

#[test]
fn renaming_onto_an_existing_id_is_refused() {
    let mut editor = linked();
    let snapshot = editor.story().clone();

    let err = editor
        .apply(&EditOp::RenameNode {
            from: NodeId::from("a"),
            to: NodeId::from("b"),
        })
        .expect_err("must refuse");

    assert!(matches!(err, EditError::NodeExists(_)));
    assert_eq!(editor.story(), &snapshot);
}

#[test]
fn renaming_a_node_to_itself_is_a_no_op_not_an_error() {
    let mut editor = linked();
    editor
        .apply(&EditOp::RenameNode {
            from: NodeId::from("a"),
            to: NodeId::from("a"),
        })
        .expect("harmless");
    assert_eq!(editor.story().nodes.len(), 2);
}

#[test]
fn duplicating_copies_beats_and_exit_but_not_references() {
    let mut editor = linked();

    editor
        .apply(&EditOp::DuplicateNode {
            id: NodeId::from("a"),
            as_id: NodeId::from("a_copy"),
        })
        .expect("duplicates");

    let story = editor.story();
    let original = &story.nodes[&NodeId::from("a")];
    let copy = &story.nodes[&NodeId::from("a_copy")];
    assert_eq!(original.beats, copy.beats);
    assert_eq!(original.exit, copy.exit);
    assert_eq!(
        story.start.as_str(),
        "a",
        "duplication must not move the start node"
    );
}

#[test]
fn beats_can_be_inserted_updated_moved_and_deleted() {
    let mut editor = linked();
    let node = NodeId::from("b");

    editor
        .apply(&EditOp::InsertBeat {
            node: node.clone(),
            index: 0,
            beat: say("zero"),
        })
        .expect("inserts at the front");

    assert_eq!(
        editor.story().nodes[&node].beats,
        vec![say("zero"), say("second")]
    );

    editor
        .apply(&EditOp::UpdateBeat {
            node: node.clone(),
            index: 1,
            beat: say("changed"),
        })
        .expect("updates");

    assert_eq!(editor.story().nodes[&node].beats[1], say("changed"));

    editor
        .apply(&EditOp::MoveBeat {
            node: node.clone(),
            from: 1,
            to: 0,
        })
        .expect("moves");

    assert_eq!(
        editor.story().nodes[&node].beats,
        vec![say("changed"), say("zero")]
    );

    editor
        .apply(&EditOp::DeleteBeat {
            node: node.clone(),
            index: 0,
        })
        .expect("deletes");

    assert_eq!(editor.story().nodes[&node].beats, vec![say("zero")]);
}

#[test]
fn inserting_a_beat_past_the_end_is_refused() {
    let mut editor = linked();
    let snapshot = editor.story().clone();

    let err = editor
        .apply(&EditOp::InsertBeat {
            node: NodeId::from("b"),
            index: 5,
            beat: say("x"),
        })
        .expect_err("must refuse");

    assert!(matches!(err, EditError::BeatOutOfRange { .. }));
    assert_eq!(editor.story(), &snapshot);
}

#[test]
fn appending_a_beat_at_exactly_the_length_is_allowed() {
    let mut editor = linked();
    editor
        .apply(&EditOp::InsertBeat {
            node: NodeId::from("b"),
            index: 1,
            beat: say("appended"),
        })
        .expect("index == len appends");
    assert_eq!(editor.story().nodes[&NodeId::from("b")].beats.len(), 2);
}

#[test]
fn moving_a_beat_to_the_end_lands_at_the_end() {
    let mut editor = editor_of(story_with(
        vec![(
            "n",
            Node {
                beats: vec![say("one"), say("two"), say("three")],
                exit: Exit::End { ending: None },
            },
        )],
        "n",
    ));

    editor
        .apply(&EditOp::MoveBeat {
            node: NodeId::from("n"),
            from: 0,
            to: 3,
        })
        .expect("moves to the end");

    let beats = &editor.story().nodes[&NodeId::from("n")].beats;
    assert_eq!(beats, &vec![say("two"), say("three"), say("one")]);
}

#[test]
fn setting_an_exit_that_points_at_a_missing_node_is_rolled_back() {
    let mut editor = linked();
    let snapshot = editor.story().clone();

    let err = editor
        .apply(&EditOp::SetExit {
            node: NodeId::from("a"),
            exit: Exit::Goto {
                to: NodeId::from("nowhere"),
            },
        })
        .expect_err("validation must catch this");

    assert!(
        matches!(err, EditError::WouldInvalidate(_)),
        "unexpected error: {err}"
    );
    assert_eq!(
        editor.story(),
        &snapshot,
        "a structurally valid operation that produces an invalid story must not stick"
    );
}

// --- choices ------------------------------------------------------------

fn choosing() -> Editor {
    editor_of(story_with(
        vec![
            (
                "ask",
                Node {
                    beats: vec![],
                    exit: Exit::Choose {
                        prompt: Some("well?".into()),
                        choices: vec![
                            Choice {
                                label: "yes".into(),
                                to: NodeId::from("yes"),
                            },
                            Choice {
                                label: "no".into(),
                                to: NodeId::from("no"),
                            },
                        ],
                    },
                },
            ),
            (
                "yes",
                Node {
                    beats: vec![],
                    exit: Exit::End { ending: None },
                },
            ),
            (
                "no",
                Node {
                    beats: vec![],
                    exit: Exit::End { ending: None },
                },
            ),
        ],
        "ask",
    ))
}

#[test]
fn a_choice_can_be_added_relabelled_and_removed() {
    let mut editor = choosing();
    let node = NodeId::from("ask");

    editor
        .apply(&EditOp::AddChoice {
            node: node.clone(),
            label: "maybe".into(),
            to: NodeId::from("yes"),
        })
        .expect("adds");

    let Exit::Choose { choices, .. } = &editor.story().nodes[&node].exit else {
        panic!("still a choice");
    };
    assert_eq!(choices.len(), 3);

    editor
        .apply(&EditOp::UpdateChoice {
            node: node.clone(),
            index: 2,
            label: Some("perhaps".into()),
            to: None,
        })
        .expect("relabels without touching the destination");

    let Exit::Choose { choices, .. } = &editor.story().nodes[&node].exit else {
        panic!("still a choice");
    };
    assert_eq!(choices[2].label, "perhaps");
    assert_eq!(choices[2].to.as_str(), "yes", "destination left alone");

    editor
        .apply(&EditOp::DeleteChoice {
            node: node.clone(),
            index: 2,
        })
        .expect("deletes");
    let Exit::Choose { choices, .. } = &editor.story().nodes[&node].exit else {
        panic!("still a choice");
    };
    assert_eq!(choices.len(), 2);
}

#[test]
fn adding_a_choice_to_a_node_that_does_not_offer_them_is_refused() {
    let mut editor = linked();
    let err = editor
        .apply(&EditOp::AddChoice {
            node: NodeId::from("b"),
            label: "x".into(),
            to: NodeId::from("a"),
        })
        .expect_err("must refuse");
    assert!(matches!(err, EditError::NoChoices(_)));
}

#[test]
fn deleting_the_last_choice_is_refused() {
    let mut editor = editor_of(story_with(
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
    ));

    let err = editor
        .apply(&EditOp::DeleteChoice {
            node: NodeId::from("ask"),
            index: 0,
        })
        .expect_err("must refuse");

    assert!(
        matches!(err, EditError::CannotDeleteLastChoice { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn retargeting_a_choice_onto_a_missing_node_is_rolled_back() {
    let mut editor = choosing();
    let snapshot = editor.story().clone();

    let err = editor
        .apply(&EditOp::UpdateChoice {
            node: NodeId::from("ask"),
            index: 0,
            label: None,
            to: Some(NodeId::from("ghost")),
        })
        .expect_err("must refuse");

    assert!(matches!(err, EditError::WouldInvalidate(_)));
    assert_eq!(editor.story(), &snapshot);
}

// --- history ------------------------------------------------------------

#[test]
fn undo_restores_the_previous_story_and_redo_replays_it() {
    let mut editor = linked();
    let original = editor.story().clone();

    editor
        .apply(&EditOp::CreateNode {
            id: NodeId::from("ghost"),
            at: None,
        })
        .expect("creates");
    let after = editor.story().clone();

    assert!(editor.can_undo());
    let undone = editor.undo().expect("undo");
    assert!(undone.contains("ghost"), "undo should say what it undid");
    assert_eq!(editor.story(), &original);

    assert!(editor.can_redo());
    editor.redo().expect("redo");
    assert_eq!(editor.story(), &after);
}

#[test]
fn a_new_edit_clears_the_redo_branch() {
    let mut editor = linked();
    editor
        .apply(&EditOp::CreateNode {
            id: NodeId::from("one"),
            at: None,
        })
        .expect("creates");
    editor.undo().expect("undo");
    assert!(editor.can_redo());

    editor
        .apply(&EditOp::CreateNode {
            id: NodeId::from("two"),
            at: None,
        })
        .expect("creates");

    assert!(
        !editor.can_redo(),
        "editing after undo must discard the abandoned future"
    );
}

#[test]
fn a_failed_operation_does_not_enter_history() {
    let mut editor = linked();
    let _ = editor.apply(&EditOp::CreateNode {
        id: NodeId::from("a"),
        at: None,
    });

    assert!(
        !editor.can_undo(),
        "nothing was applied, so there is nothing to undo"
    );
}

#[test]
fn a_batch_rolls_back_entirely_when_a_later_operation_fails() {
    let mut editor = linked();
    let snapshot = editor.story().clone();

    let ops = [
        EditOp::CreateNode {
            id: NodeId::from("new1"),
            at: None,
        },
        EditOp::CreateNode {
            id: NodeId::from("new2"),
            at: None,
        },
        // Fails: the id already exists.
        EditOp::CreateNode {
            id: NodeId::from("a"),
            at: None,
        },
    ];

    let err = editor.apply_all(&ops).expect_err("must fail");
    assert_eq!(err.index, 2, "the error must name the failing operation");
    assert_eq!(
        editor.story(),
        &snapshot,
        "a batch failure must not leave the successful prefix applied"
    );
    assert!(!editor.can_undo(), "and must not leave history behind");
}

#[test]
fn a_successful_batch_applies_every_operation() {
    let mut editor = linked();
    let ops = [
        EditOp::CreateNode {
            id: NodeId::from("x"),
            at: None,
        },
        EditOp::CreateNode {
            id: NodeId::from("y"),
            at: None,
        },
    ];

    assert_eq!(editor.apply_all(&ops).expect("batch"), 2);
    assert!(editor.story().nodes.contains_key(&NodeId::from("x")));
    assert!(editor.story().nodes.contains_key(&NodeId::from("y")));
}

#[test]
fn metadata_edits_are_ordinary_operations() {
    let mut editor = linked();
    editor
        .apply(&EditOp::SetTitle {
            title: Some("A Story".into()),
        })
        .expect("sets");
    assert_eq!(editor.story().meta.title.as_deref(), Some("A Story"));

    editor
        .apply(&EditOp::SetTitle { title: None })
        .expect("clears");
    assert_eq!(editor.story().meta.title, None);
}

#[test]
fn the_start_node_can_be_moved_to_any_existing_node() {
    let mut editor = linked();
    editor
        .apply(&EditOp::SetStartNode {
            node: NodeId::from("b"),
        })
        .expect("sets");
    assert_eq!(editor.story().start.as_str(), "b");
}

#[test]
fn moving_the_start_node_to_a_missing_node_is_refused() {
    let mut editor = linked();
    let err = editor
        .apply(&EditOp::SetStartNode {
            node: NodeId::from("ghost"),
        })
        .expect_err("must refuse");
    assert!(matches!(err, EditError::NoSuchNode(_)));
}

// --- problems() ---------------------------------------------------------

#[test]
fn problems_reports_every_fault_not_just_the_first() {
    // Hand-built invalid story: two independent dangling references.
    let broken = story_with(
        vec![
            (
                "a",
                Node {
                    beats: vec![],
                    exit: Exit::Goto {
                        to: NodeId::from("gone1"),
                    },
                },
            ),
            (
                "b",
                Node {
                    beats: vec![],
                    exit: Exit::Goto {
                        to: NodeId::from("gone2"),
                    },
                },
            ),
        ],
        "a",
    );

    let editor = Editor::new(Document::new(broken, "broken.ron"));
    let problems = editor.problems();

    // Three faults: two dangling references, and `b` — which nothing can reach because
    // `a` points at a node that does not exist.
    assert_eq!(problems.len(), 3, "got {problems:#?}");
    assert!(problems.iter().any(|p| p.contains("gone1")));
    assert!(problems.iter().any(|p| p.contains("gone2")));
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`b`") && p.contains("reached")),
        "got {problems:#?}"
    );
}

#[test]
fn a_node_nothing_points_at_is_reported_as_unreachable() {
    let mut editor = linked();
    editor
        .apply(&EditOp::CreateNode {
            id: NodeId::from("orphan"),
            at: None,
        })
        .expect("create");

    let problems = editor.problems();
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("orphan") && problem.contains("cannot be reached")),
        "got {problems:#?}"
    );
}

#[test]
fn a_valid_story_reports_no_problems() {
    let editor = linked();
    assert!(editor.problems().is_empty(), "{:#?}", editor.problems());
}

#[test]
fn dirty_tracks_unsaved_changes() {
    let mut editor = linked();
    assert!(!editor.is_dirty(), "a freshly built editor is clean");

    editor
        .apply(&EditOp::CreateNode {
            id: NodeId::from("z"),
            at: None,
        })
        .expect("creates");
    assert!(editor.is_dirty());

    let dir = std::env::temp_dir().join("rustory-edit-dirty");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("story.ron");

    editor.save_as(&path).expect("saves");
    assert!(!editor.is_dirty(), "saving clears the flag");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_saved_file_loads_back_with_every_beat_intact() {
    let dir = std::env::temp_dir().join("rustory-edit-roundtrip");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("story.ron");

    let mut editor = editor_of(story_with(
        vec![(
            "scene",
            Node {
                beats: vec![
                    Beat::SetBackground {
                        image: Some("bg.png".into()),
                    },
                    Beat::ShowActor {
                        actor: ActorId::from("yuki"),
                        pose: "smile".into(),
                        slot: Slot::Left,
                    },
                    say("a line with 中文 and an emoji 🎭"),
                ],
                exit: Exit::Choose {
                    prompt: Some("and then?".into()),
                    choices: vec![Choice {
                        label: "onwards".into(),
                        to: NodeId::from("scene"),
                    }],
                },
            },
        )],
        "scene",
    ));

    editor.save_as(&path).expect("saves");

    let reopened = Editor::open(&path).expect("reopens");
    assert_eq!(
        reopened.story(),
        editor.story(),
        "saving must be lossless, including non-ASCII text"
    );

    let _ = std::fs::remove_file(&path);
}

// --- gameplay outcomes --------------------------------------------------

/// A node that hands over to a system with two mapped outcomes.
fn gameplay() -> Editor {
    editor_of(story_with(
        vec![
            (
                "play",
                Node {
                    beats: vec![],
                    exit: Exit::Gameplay {
                        spec: GameplaySpec {
                            system: SystemId::from("timer.timed_choice"),
                            config: None,
                            outcomes: [
                                (OutcomeKey::from("correct"), NodeId::from("won")),
                                (OutcomeKey::from("timeout"), NodeId::from("lost")),
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
                    beats: vec![],
                    exit: Exit::End { ending: None },
                },
            ),
            (
                "lost",
                Node {
                    beats: vec![],
                    exit: Exit::End { ending: None },
                },
            ),
        ],
        "play",
    ))
}

fn outcomes_of(editor: &Editor, node: &str) -> Vec<(String, String)> {
    let Exit::Gameplay { spec } = &editor.story().nodes[&NodeId::from(node)].exit else {
        panic!("`{node}` should be a gameplay node");
    };
    spec.outcomes
        .iter()
        .map(|(key, to)| (key.to_string(), to.to_string()))
        .collect()
}

#[test]
fn an_outcome_mapping_can_be_added() {
    let mut editor = gameplay();

    editor
        .apply(&EditOp::SetOutcome {
            node: NodeId::from("play"),
            outcome: OutcomeKey::from("wrong"),
            to: NodeId::from("lost"),
        })
        .expect("adds");

    let outcomes = outcomes_of(&editor, "play");
    assert_eq!(outcomes.len(), 3);
    assert!(
        outcomes.contains(&("wrong".into(), "lost".into())),
        "got {outcomes:?}"
    );
}

#[test]
fn setting_an_existing_outcome_retargets_it_rather_than_duplicating() {
    let mut editor = gameplay();

    editor
        .apply(&EditOp::SetOutcome {
            node: NodeId::from("play"),
            outcome: OutcomeKey::from("correct"),
            to: NodeId::from("lost"),
        })
        .expect("retargets");

    let outcomes = outcomes_of(&editor, "play");
    assert_eq!(outcomes.len(), 2, "a key cannot appear twice: {outcomes:?}");
    assert!(outcomes.contains(&("correct".into(), "lost".into())));
}

#[test]
fn an_outcome_mapping_can_be_removed() {
    let mut editor = gameplay();

    editor
        .apply(&EditOp::DeleteOutcome {
            node: NodeId::from("play"),
            outcome: OutcomeKey::from("timeout"),
        })
        .expect("removes");

    let outcomes = outcomes_of(&editor, "play");
    assert_eq!(outcomes, vec![("correct".to_string(), "won".to_string())]);
}

#[test]
fn removing_an_outcome_that_does_not_exist_is_refused_by_name() {
    let mut editor = gameplay();
    let snapshot = editor.story().clone();

    let err = editor
        .apply(&EditOp::DeleteOutcome {
            node: NodeId::from("play"),
            outcome: OutcomeKey::from("ghost"),
        })
        .expect_err("must refuse");

    assert!(
        matches!(err, EditError::NoSuchOutcome { ref outcome, .. } if outcome == "ghost"),
        "unexpected error: {err}"
    );
    assert_eq!(editor.story(), &snapshot, "a refused edit changes nothing");
}

#[test]
fn mapping_an_outcome_to_a_missing_node_is_rolled_back() {
    let mut editor = gameplay();
    let snapshot = editor.story().clone();

    let err = editor
        .apply(&EditOp::SetOutcome {
            node: NodeId::from("play"),
            outcome: OutcomeKey::from("correct"),
            to: NodeId::from("nowhere"),
        })
        .expect_err("validation must catch the dangling target");

    assert!(matches!(err, EditError::WouldInvalidate(_)));
    assert_eq!(editor.story(), &snapshot);
}

#[test]
fn outcomes_can_only_be_set_on_a_gameplay_node() {
    let mut editor = linked();
    let err = editor
        .apply(&EditOp::SetOutcome {
            node: NodeId::from("b"),
            outcome: OutcomeKey::from("win"),
            to: NodeId::from("a"),
        })
        .expect_err("must refuse");
    assert!(matches!(err, EditError::NoGameplay(_)));
}

#[test]
fn emptying_the_outcome_table_is_allowed_but_reported_as_a_problem() {
    // Deleting mappings one by one is how an author retargets a scene. Blocking the
    // last deletion would trap them; the fault list is what warns them.
    let mut editor = gameplay();
    editor
        .apply(&EditOp::DeleteOutcome {
            node: NodeId::from("play"),
            outcome: OutcomeKey::from("correct"),
        })
        .expect("first removal");
    editor
        .apply(&EditOp::DeleteOutcome {
            node: NodeId::from("play"),
            outcome: OutcomeKey::from("timeout"),
        })
        .expect("second removal");

    assert!(outcomes_of(&editor, "play").is_empty());
    assert!(
        editor
            .problems()
            .iter()
            .any(|p| p.contains("maps no outcomes")),
        "an unmapped gameplay node should be flagged: {:#?}",
        editor.problems()
    );
}

// --- undo coalescing ----------------------------------------------------

/// Typing into a field must not leave one undo step per keystroke.
///
/// Without merging, the sixth Ctrl+Z deletes a letter rather than the sentence, which is
/// worse than no undo at all because the user has to count.
#[test]
fn consecutive_edits_to_one_beat_merge_into_a_single_undo_step() {
    let mut editor = linked();
    let before = editor.story().clone();
    let node = NodeId::from("b");

    for text in ["h", "he", "hel", "hell", "hello"] {
        editor
            .apply(&EditOp::UpdateBeat {
                node: node.clone(),
                index: 0,
                beat: say(text),
            })
            .expect("applies");
    }

    assert_eq!(
        editor.story().nodes[&node].beats[0],
        say("hello"),
        "the last keystroke wins"
    );

    // One undo must go back to before the typing started, not to "hell".
    editor.undo().expect("undo");
    assert_eq!(
        editor.story(),
        &before,
        "undo should step over the whole word, not one character"
    );
}

#[test]
fn editing_a_different_beat_starts_a_new_undo_step() {
    let mut editor = linked();
    let node = NodeId::from("b");

    editor
        .apply(&EditOp::UpdateBeat {
            node: node.clone(),
            index: 0,
            beat: say("first field"),
        })
        .expect("first");
    editor
        .apply(&EditOp::UpdateBeat {
            node: node.clone(),
            index: 0,
            beat: say("first field, edited"),
        })
        .expect("continues");

    // Different target: must not merge.
    editor
        .apply(&EditOp::InsertBeat {
            node: node.clone(),
            index: 1,
            beat: say("second"),
        })
        .expect("inserts");
    editor
        .apply(&EditOp::UpdateBeat {
            node: node.clone(),
            index: 1,
            beat: say("second, edited"),
        })
        .expect("edits");

    assert_eq!(editor.story().nodes[&node].beats.len(), 2);

    // Three distinct steps: beat 1 typing merged, insert, beat 2 typing merged.
    editor.undo().expect("undo 1");
    assert_eq!(editor.story().nodes[&node].beats[1], say("second"));
    editor.undo().expect("undo 2");
    assert_eq!(editor.story().nodes[&node].beats.len(), 1);
    editor.undo().expect("undo 3");
    assert_eq!(editor.story().nodes[&node].beats[0], say("second"));
}

#[test]
fn undo_clears_the_typing_chain_so_the_next_edit_is_its_own_step() {
    let mut editor = linked();
    let node = NodeId::from("b");

    editor
        .apply(&EditOp::UpdateBeat {
            node: node.clone(),
            index: 0,
            beat: say("typed"),
        })
        .expect("first");
    editor.undo().expect("undo");

    editor
        .apply(&EditOp::UpdateBeat {
            node: node.clone(),
            index: 0,
            beat: say("typed again"),
        })
        .expect("second");

    // If the chain had survived the undo, this undo would jump past the first edit too.
    editor.undo().expect("undo");
    assert_eq!(editor.story().nodes[&node].beats[0], say("second"));
}

#[test]
fn a_batch_gives_each_entry_its_own_undo_step() {
    // A caller listing ten edits in a batch expects ten steps, not one merged blob.
    let mut editor = linked();
    let ops = [
        EditOp::CreateNode {
            id: NodeId::from("x"),
            at: None,
        },
        EditOp::CreateNode {
            id: NodeId::from("y"),
            at: None,
        },
    ];

    editor.apply_all(&ops).expect("batch");
    editor.undo().expect("undo 1");
    assert!(editor.story().nodes.contains_key(&NodeId::from("x")));
    assert!(!editor.story().nodes.contains_key(&NodeId::from("y")));
}

#[test]
fn title_typing_merges_but_a_different_field_does_not() {
    let mut editor = linked();

    for title in ["A", "A S", "A St", "A Story"] {
        editor
            .apply(&EditOp::SetTitle {
                title: Some(title.into()),
            })
            .expect("applies");
    }
    editor
        .apply(&EditOp::SetAuthor {
            author: Some("me".into()),
        })
        .expect("applies");

    assert_eq!(editor.story().meta.title.as_deref(), Some("A Story"));

    editor.undo().expect("undo author");
    assert_eq!(editor.story().meta.author, None, "author is its own step");
    assert_eq!(
        editor.story().meta.title.as_deref(),
        Some("A Story"),
        "title survived"
    );

    editor.undo().expect("undo title");
    assert_eq!(
        editor.story().meta.title,
        None,
        "the whole title is one step"
    );
}

#[test]
fn retyping_an_ending_name_is_one_undo_step() {
    let mut editor = linked();
    let node = NodeId::from("b");

    for ending in ["h", "ha", "hap", "happy"] {
        editor
            .apply(&EditOp::SetExit {
                node: node.clone(),
                exit: Exit::End {
                    ending: Some(ending.into()),
                },
            })
            .expect("applies");
    }

    assert_eq!(
        editor.story().nodes[&node].exit,
        Exit::End {
            ending: Some("happy".into())
        }
    );

    editor.undo().expect("undo");
    assert_eq!(
        editor.story().nodes[&node].exit,
        Exit::End { ending: None },
        "the exit form is one field, so one undo step"
    );
}

// --- choice order -------------------------------------------------------

/// `pick` offers three options to `a`, `b` and `c`.
fn picky() -> Editor {
    editor_of(story_with(
        vec![
            (
                "pick",
                Node {
                    beats: vec![say("choose")],
                    exit: Exit::Choose {
                        prompt: None,
                        choices: vec![
                            Choice {
                                label: "first".into(),
                                to: NodeId::from("a"),
                            },
                            Choice {
                                label: "second".into(),
                                to: NodeId::from("b"),
                            },
                            Choice {
                                label: "third".into(),
                                to: NodeId::from("c"),
                            },
                        ],
                    },
                },
            ),
            (
                "a",
                Node {
                    beats: vec![],
                    exit: Exit::End { ending: None },
                },
            ),
            (
                "b",
                Node {
                    beats: vec![],
                    exit: Exit::End { ending: None },
                },
            ),
            (
                "c",
                Node {
                    beats: vec![],
                    exit: Exit::End { ending: None },
                },
            ),
        ],
        "pick",
    ))
}

fn labels(editor: &Editor) -> Vec<String> {
    match &editor.story().nodes[&NodeId::from("pick")].exit {
        Exit::Choose { choices, .. } => choices.iter().map(|c| c.label.clone()).collect(),
        other => panic!("expected a choice exit, got {other:?}"),
    }
}

#[test]
fn moving_a_choice_down_swaps_it_with_the_next_one() {
    let mut editor = picky();
    editor
        .apply(&EditOp::MoveChoice {
            node: NodeId::from("pick"),
            from: 0,
            to: 2,
        })
        .expect("moves");

    // `to` is the index the option would occupy before the move, exactly as for beats, so
    // moving the first option to 2 puts it in the middle.
    assert_eq!(labels(&editor), ["second", "first", "third"]);
}

#[test]
fn moving_a_choice_up_swaps_it_with_the_previous_one() {
    let mut editor = picky();
    editor
        .apply(&EditOp::MoveChoice {
            node: NodeId::from("pick"),
            from: 2,
            to: 0,
        })
        .expect("moves");

    assert_eq!(labels(&editor), ["third", "first", "second"]);
}

#[test]
fn moving_a_choice_to_the_end_appends_it() {
    let mut editor = picky();
    editor
        .apply(&EditOp::MoveChoice {
            node: NodeId::from("pick"),
            from: 0,
            to: 3,
        })
        .expect("moves");

    assert_eq!(labels(&editor), ["second", "third", "first"]);
}

#[test]
fn moving_a_choice_out_of_range_is_refused_and_leaves_the_story_alone() {
    let mut editor = picky();
    let before = editor.story().clone();

    let err = editor
        .apply(&EditOp::MoveChoice {
            node: NodeId::from("pick"),
            from: 3,
            to: 0,
        })
        .expect_err("out of range");
    assert!(matches!(err, EditError::ChoiceOutOfRange { .. }), "{err}");
    assert_eq!(editor.story(), &before, "a refusal changes nothing");

    let err = editor
        .apply(&EditOp::MoveChoice {
            node: NodeId::from("pick"),
            from: 0,
            to: 4,
        })
        .expect_err("beyond the end");
    assert!(matches!(err, EditError::ChoiceOutOfRange { .. }), "{err}");
}

#[test]
fn reordering_a_node_that_offers_no_choices_is_refused() {
    let mut editor = linked();
    let err = editor
        .apply(&EditOp::MoveChoice {
            node: NodeId::from("a"),
            from: 0,
            to: 1,
        })
        .expect_err("a goto has no options");
    assert!(matches!(err, EditError::NoChoices(_)), "{err}");
}

// --- gameplay config ----------------------------------------------------

#[test]
fn a_gameplay_config_can_be_set_and_read_back() {
    let mut editor = gameplay();
    let node = NodeId::from("play");
    assert_eq!(gameplay_spec(&editor).config, None);

    editor
        .apply(&EditOp::SetGameplayConfig {
            node: node.clone(),
            config: Some("(duration_ms: 3000)".into()),
        })
        .expect("applies");

    let config = gameplay_spec(&editor).config.clone().expect("config set");
    // Compared as text rather than by shape: `ron::Value` is the point — the editor has no
    // opinion about the type, only that it parses.
    let text = ron::ser::to_string(&config).expect("serialises");
    assert!(text.contains("duration_ms"), "{text}");
    assert!(text.contains("3000"), "{text}");
}

#[test]
fn a_broken_config_is_refused_and_the_old_one_survives() {
    let mut editor = gameplay();
    let node = NodeId::from("play");
    editor
        .apply(&EditOp::SetGameplayConfig {
            node: node.clone(),
            config: Some("(duration_ms: 3000)".into()),
        })
        .expect("applies");
    let before = editor.story().clone();

    let err = editor
        .apply(&EditOp::SetGameplayConfig {
            node: node.clone(),
            config: Some("(duration_ms: ".into()),
        })
        .expect_err("unbalanced parens must not be accepted");

    assert!(matches!(err, EditError::BadConfig { .. }), "{err}");
    assert_eq!(
        editor.story(),
        &before,
        "a refused config must not clear the working one"
    );
}

#[test]
fn an_empty_config_clears_it() {
    let mut editor = gameplay();
    let node = NodeId::from("play");
    editor
        .apply(&EditOp::SetGameplayConfig {
            node: node.clone(),
            config: Some("(duration_ms: 3000)".into()),
        })
        .expect("applies");
    assert!(gameplay_spec(&editor).config.is_some());

    // Empty text from a text box that the author emptied, not an explicit `None`.
    editor
        .apply(&EditOp::SetGameplayConfig {
            node: node.clone(),
            config: Some("   ".into()),
        })
        .expect("clears");
    assert_eq!(gameplay_spec(&editor).config, None);

    editor
        .apply(&EditOp::SetGameplayConfig {
            node: node.clone(),
            config: Some("(x: 1)".into()),
        })
        .expect("applies");
    editor
        .apply(&EditOp::SetGameplayConfig { node, config: None })
        .expect("clears");
    assert_eq!(gameplay_spec(&editor).config, None);
}

#[test]
fn a_config_can_only_be_set_on_a_gameplay_node() {
    let mut editor = linked();
    let err = editor
        .apply(&EditOp::SetGameplayConfig {
            node: NodeId::from("a"),
            config: Some("(x: 1)".into()),
        })
        .expect_err("a goto has no config");
    assert!(matches!(err, EditError::NoGameplay(_)), "{err}");
}

fn gameplay_spec(editor: &Editor) -> &GameplaySpec {
    match &editor.story().nodes[&NodeId::from("play")].exit {
        Exit::Gameplay { spec } => spec,
        other => panic!("expected a gameplay exit, got {other:?}"),
    }
}
