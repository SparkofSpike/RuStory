//! The editing timeline: the list a history panel shows, and the jump it performs.
//!
//! The theme: a jump is not a second history mechanism. It is undo or redo repeated, so it
//! cannot disagree with the keyboard shortcuts about what the document contains.

use rustory_vn_core::{Beat, Exit, Node, NodeId, Story};
use rustory_vn_edit::prelude::*;
use rustory_vn_edit::{Document, Editor};

fn say(text: &str) -> Beat {
    Beat::Say {
        speaker: None,
        text: text.into(),
    }
}

fn editor() -> Editor {
    let mut nodes = std::collections::BTreeMap::new();
    nodes.insert(
        NodeId::from("start"),
        Node {
            beats: vec![say("one")],
            exit: Exit::End { ending: None },
        },
    );
    Editor::new(Document::new(Story::new("start", nodes), "test.ron"))
}

/// Three operations that do not coalesce: title, author, and a new node.
fn edited() -> Editor {
    let mut editor = editor();
    editor
        .apply(&EditOp::SetTitle {
            title: Some("First".into()),
        })
        .expect("set title");
    editor
        .apply(&EditOp::SetAuthor {
            author: Some("A".into()),
        })
        .expect("set author");
    editor
        .apply(&EditOp::CreateNode {
            id: NodeId::from("second"),
            at: None,
        })
        .expect("create node");
    editor
}

#[test]
fn a_fresh_editor_has_one_point_and_it_is_current() {
    let editor = editor();
    let timeline = editor.timeline();

    assert_eq!(timeline.len(), 1);
    assert_eq!(timeline[0].index, 0);
    assert!(timeline[0].label.is_empty(), "no operation produced it");
    assert!(timeline[0].current);
    assert_eq!(editor.timeline_position(), 0);
}

#[test]
fn each_operation_adds_a_point_and_the_last_one_is_current() {
    let editor = edited();
    let timeline = editor.timeline();

    assert_eq!(timeline.len(), 4, "the opening state plus three edits");
    assert_eq!(
        timeline.iter().map(|p| p.index).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert_eq!(timeline[1].label, "set title");
    assert_eq!(timeline[2].label, "set author");
    assert_eq!(timeline[3].label, "create node second");
    assert!(timeline[3].current);
    assert!(timeline[..3].iter().all(|p| !p.current));
    assert_eq!(editor.timeline_position(), 3);
}

#[test]
fn jumping_back_rewinds_the_document_and_marks_the_new_point() {
    let mut editor = edited();
    editor.goto(1).expect("point 1 is reachable");

    assert_eq!(editor.timeline_position(), 1);
    assert_eq!(
        editor.story().meta.title.as_deref(),
        Some("First"),
        "the title survived, the author and the node did not"
    );
    assert_eq!(editor.story().meta.author, None);
    assert!(!editor.story().nodes.contains_key(&NodeId::from("second")));

    let timeline = editor.timeline();
    assert_eq!(timeline.len(), 4, "jumping does not throw the future away");
    assert!(timeline[1].current);
}

#[test]
fn jumping_forward_reapplies_what_was_undone() {
    let mut editor = edited();
    editor.goto(1).expect("back");
    editor.goto(3).expect("forward again");

    assert_eq!(editor.timeline_position(), 3);
    assert_eq!(editor.story().meta.title.as_deref(), Some("First"));
    assert_eq!(editor.story().meta.author.as_deref(), Some("A"));
    assert!(editor.story().nodes.contains_key(&NodeId::from("second")));
}

#[test]
fn jumping_to_where_you_already_are_changes_nothing() {
    let mut editor = edited();
    let before = editor.story().clone();

    editor.goto(3).expect("same point");

    assert_eq!(editor.story(), &before);
    assert_eq!(editor.timeline_position(), 3);
}

#[test]
fn a_jump_past_the_end_is_refused_and_leaves_the_document_alone() {
    let mut editor = edited();
    let before = editor.story().clone();

    let error = editor.goto(9).expect_err("only 0..=3 exist");

    assert_eq!(error.code(), "no_such_point");
    assert_eq!(editor.story(), &before);
    assert_eq!(editor.timeline_position(), 3);
}

#[test]
fn an_edit_after_a_jump_truncates_the_timeline() {
    // The same rule undo follows: a new edit invalidates the branch that was stepped back
    // from. A history panel showing stale future rows would offer a jump that cannot happen.
    let mut editor = edited();
    editor.goto(1).expect("back");
    assert_eq!(editor.timeline().len(), 4);

    editor
        .apply(&EditOp::SetStartNode {
            node: NodeId::from("start"),
        })
        .expect("set start");

    let timeline = editor.timeline();
    assert_eq!(timeline.len(), 3, "opening state, title, and the new edit");
    assert!(timeline[2].current);
    assert_eq!(timeline[2].label, "start at start");
}

#[test]
fn a_timeline_of_a_long_session_stays_in_step_with_undo() {
    // Points, positions and undo must agree after arbitrary jumps. Ten edits, ten jumps,
    // comparing at every stop against what plain undo would have produced.
    let mut editor = editor();
    for n in 0..10 {
        editor
            .apply(&EditOp::SetTitle {
                title: Some(format!("t{n}")),
            })
            .expect("set title");
        // Separate the operations so the coalescing rule does not merge them.
        editor
            .apply(&EditOp::SetAuthor {
                author: Some(format!("a{n}")),
            })
            .expect("set author");
    }
    assert_eq!(editor.timeline_position(), 20);

    for target in [0, 20, 5, 17, 1, 19] {
        editor.goto(target).expect("reachable");
        assert_eq!(editor.timeline_position(), target);

        let mut expected = editor.story().clone();
        // Walking forward from the same point with redo must give the same document.
        while editor.timeline_position() > 0 {
            editor.undo();
        }
        while editor.timeline_position() < target {
            editor.redo();
        }
        assert_eq!(editor.story(), &expected);
        expected = editor.story().clone();
        assert_eq!(editor.timeline_position(), target);
    }
}
