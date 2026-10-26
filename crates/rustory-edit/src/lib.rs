//! Editing operations, and the document they act on.
//!
//! # Why this crate exists
//!
//! An editor is not a window with widgets in it. An editor is **a set of typed
//! operations on a document**, plus however many front-ends you build on top. Get
//! the operations right and a GUI and a command line are both thin; get them wrong
//! and you end up with two programs that disagree about what a story is.
//!
//! So this crate owns:
//!
//! * [`Document`] — the thing being edited, which is a [`Story`] plus where it came
//!   from.
//! * [`EditOp`] — every change a human or an agent can make, as one enum.
//! * [`Editor`] — applies operations, validates the result, and either keeps the
//!   change or rolls it back.
//!
//! # The guarantee that makes agents safe to point at it
//!
//! [`Editor::apply`] either succeeds and leaves a valid document, or fails and
//! leaves the document **exactly** as it was. There is no third outcome. An agent
//! that issues a bad operation gets an error naming what was wrong, and the file on
//! disk is untouched until someone calls [`Editor::save`].
//!
//! This is why deletion refuses to create dangling references instead of quietly
//! repointing them: an operation that "works" by guessing is worse than one that
//! explains why it cannot.
//!
//! # Undo
//!
//! Snapshots, not inverse operations. A story is kilobytes and computing the inverse
//! of every operation is a second implementation of every operation, with its own
//! bugs. ADR 0006 chose snapshots over replay for saves for the same reason.

#![forbid(unsafe_code)]

mod apply;
mod document;
mod editor;
mod error;
mod op;

pub use document::Document;
pub use editor::{BatchError, Editor, HISTORY_LIMIT, HistoryEntry};
pub use error::EditError;
pub use op::EditOp;

/// Convenience re-exports for callers that only manipulate stories.
pub mod prelude {
    pub use crate::op::{BeatLocation, EditOp, NodeView};
    pub use crate::{Document, EditError, Editor, HistoryEntry};
    pub use rustory_core::{
        ActorId, Beat, Choice, Exit, GameplaySpec, Node, NodeId, OutcomeKey, Slot, Story,
        StoryMeta, SystemId,
    };
}

/// The shape of an operation as an agent sees it.
///
/// `rustory ops --json` prints one entry per operation so a caller can discover the
/// vocabulary instead of being told it out of band. Kept next to the enum so adding
/// an operation without documenting it is a compile error rather than a doc drift:
/// [`catalogue`] fails to build if a variant is unhandled.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OpInfo {
    pub name: &'static str,
    pub summary: &'static str,
    pub changes_structure: bool,
}

/// Every operation, with a one-line description.
#[must_use]
pub fn catalogue() -> Vec<OpInfo> {
    use EditOp as E;

    // A match rather than a list, so a new variant forces this function to be
    // updated: `catalogue` will not compile otherwise.
    fn describe(op: &E) -> OpInfo {
        match op {
            E::SetTitle { .. } => OpInfo {
                name: "SetTitle",
                summary: "Set or clear the story title.",
                changes_structure: false,
            },
            E::SetAuthor { .. } => OpInfo {
                name: "SetAuthor",
                summary: "Set or clear the story author.",
                changes_structure: false,
            },
            E::SetStartNode { .. } => OpInfo {
                name: "SetStartNode",
                summary: "Choose which node the story begins at.",
                changes_structure: false,
            },
            E::CreateNode { .. } => OpInfo {
                name: "CreateNode",
                summary: "Add an empty node that ends the story.",
                changes_structure: true,
            },
            E::DeleteNode { .. } => OpInfo {
                name: "DeleteNode",
                summary: "Remove a node. Refused while anything still points at it.",
                changes_structure: true,
            },
            E::RenameNode { .. } => OpInfo {
                name: "RenameNode",
                summary: "Rename a node and rewrite every reference to it.",
                changes_structure: true,
            },
            E::DuplicateNode { .. } => OpInfo {
                name: "DuplicateNode",
                summary: "Copy a node's beats and exit under a new id.",
                changes_structure: true,
            },
            E::InsertBeat { .. } => OpInfo {
                name: "InsertBeat",
                summary: "Insert a beat at an index in a node.",
                changes_structure: true,
            },
            E::UpdateBeat { .. } => OpInfo {
                name: "UpdateBeat",
                summary: "Replace the beat at an index.",
                changes_structure: false,
            },
            E::DeleteBeat { .. } => OpInfo {
                name: "DeleteBeat",
                summary: "Remove the beat at an index.",
                changes_structure: true,
            },
            E::MoveBeat { .. } => OpInfo {
                name: "MoveBeat",
                summary: "Reorder beats within a node.",
                changes_structure: true,
            },
            E::SetExit { .. } => OpInfo {
                name: "SetExit",
                summary: "Replace a node's exit (end, goto, choose or gameplay).",
                changes_structure: true,
            },
            E::AddChoice { .. } => OpInfo {
                name: "AddChoice",
                summary: "Append an option to a node that offers choices.",
                changes_structure: true,
            },
            E::UpdateChoice { .. } => OpInfo {
                name: "UpdateChoice",
                summary: "Change an option's label or destination.",
                changes_structure: false,
            },
            E::DeleteChoice { .. } => OpInfo {
                name: "DeleteChoice",
                summary: "Remove an option from a node that offers choices.",
                changes_structure: true,
            },
            E::SetOutcome { .. } => OpInfo {
                name: "SetOutcome",
                summary: "Map a gameplay outcome key to a node.",
                changes_structure: true,
            },
            E::DeleteOutcome { .. } => OpInfo {
                name: "DeleteOutcome",
                summary: "Remove a gameplay outcome mapping.",
                changes_structure: true,
            },
        }
    }

    // One representative of each variant, only to drive `describe`.
    let samples = [
        E::SetTitle { title: None },
        E::SetAuthor { author: None },
        E::SetStartNode {
            node: NodeId::from("x"),
        },
        E::CreateNode {
            id: NodeId::from("x"),
            at: None,
        },
        E::DeleteNode {
            id: NodeId::from("x"),
        },
        E::RenameNode {
            from: NodeId::from("x"),
            to: NodeId::from("y"),
        },
        E::DuplicateNode {
            id: NodeId::from("x"),
            as_id: NodeId::from("y"),
        },
        E::InsertBeat {
            node: NodeId::from("x"),
            index: 0,
            beat: Beat::Say {
                speaker: None,
                text: String::new(),
            },
        },
        E::UpdateBeat {
            node: NodeId::from("x"),
            index: 0,
            beat: Beat::Say {
                speaker: None,
                text: String::new(),
            },
        },
        E::DeleteBeat {
            node: NodeId::from("x"),
            index: 0,
        },
        E::MoveBeat {
            node: NodeId::from("x"),
            from: 0,
            to: 1,
        },
        E::SetExit {
            node: NodeId::from("x"),
            exit: Exit::End { ending: None },
        },
        E::AddChoice {
            node: NodeId::from("x"),
            label: String::new(),
            to: NodeId::from("y"),
        },
        E::UpdateChoice {
            node: NodeId::from("x"),
            index: 0,
            label: None,
            to: None,
        },
        E::DeleteChoice {
            node: NodeId::from("x"),
            index: 0,
        },
        E::SetOutcome {
            node: NodeId::from("x"),
            outcome: OutcomeKey::from("win"),
            to: NodeId::from("y"),
        },
        E::DeleteOutcome {
            node: NodeId::from("x"),
            outcome: OutcomeKey::from("win"),
        },
    ];

    samples.iter().map(describe).collect()
}

use rustory_core::{Beat, Exit, NodeId, OutcomeKey};
