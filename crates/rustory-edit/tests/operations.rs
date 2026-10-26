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

    assert_eq!(problems.len(), 2, "got {problems:#?}");
    assert!(problems.iter().any(|p| p.contains("gone1")));
    assert!(problems.iter().any(|p| p.contains("gone2")));
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
