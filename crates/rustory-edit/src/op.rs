//! Every change a human or an agent can make, as one enum.
//!
//! Two properties matter and both are deliberate:
//!
//! * **Exhaustive.** Anything the GUI can do, the CLI can do, because both produce
//!   values of this type. A GUI-only affordance would mean the two front-ends had
//!   genuinely diverged.
//! * **Serialisable.** `rustory edit story.ron --op '{"op":"CreateNode",...}'` takes
//!   exactly the value the GUI would construct, so an agent and a person are
//!   editing through the same code path and cannot disagree about the result.

use rustory_core::{Beat, Choice, Exit, NodeId};
use serde::{Deserialize, Serialize};

/// A single edit.
///
/// Serde's external tagging means the JSON shape is
/// `{"op": "CreateNode", "id": "prologue", "at": null}` — the variant name is the
/// `op` field, and the fields sit beside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum EditOp {
    // --- story level -----------------------------------------------------
    /// Set or clear the title.
    SetTitle {
        #[serde(default)]
        title: Option<String>,
    },
    /// Set or clear the author.
    SetAuthor {
        #[serde(default)]
        author: Option<String>,
    },
    /// Choose the entry node. The node must already exist.
    SetStartNode { node: NodeId },

    // --- nodes -----------------------------------------------------------
    /// Add an empty node. It starts with no beats and an `End` exit, which is the
    /// only shape that is immediately valid.
    CreateNode {
        id: NodeId,
        /// Unused by the data model — `Story::nodes` is a sorted map — but recorded
        /// so a caller can express intent. Kept for symmetry with future orderable
        /// formats.
        #[serde(default)]
        at: Option<usize>,
    },
    /// Remove a node.
    ///
    /// Refused while anything references it, or while it is the start node, or while
    /// it is the only node. Repointing references automatically would be guessing
    /// where the author meant them to go.
    DeleteNode { id: NodeId },
    /// Rename a node, rewriting every reference: the start node, gotos, choice
    /// targets and gameplay outcome targets.
    RenameNode { from: NodeId, to: NodeId },
    /// Copy a node's beats and exit under a new id. References are *not* rewritten:
    /// the copy is an independent node that happens to start out identical.
    DuplicateNode { id: NodeId, as_id: NodeId },

    // --- beats -----------------------------------------------------------
    /// Insert a beat. `index` may equal the current length, which appends.
    InsertBeat {
        node: NodeId,
        index: usize,
        beat: Beat,
    },
    /// Replace the beat at `index`.
    UpdateBeat {
        node: NodeId,
        index: usize,
        beat: Beat,
    },
    /// Remove the beat at `index`.
    DeleteBeat { node: NodeId, index: usize },
    /// Move a beat within its node. Used by drag-and-drop in the GUI.
    MoveBeat {
        node: NodeId,
        from: usize,
        to: usize,
    },

    // --- exits and choices -----------------------------------------------
    /// Replace a node's exit wholesale.
    SetExit { node: NodeId, exit: Exit },
    /// Append an option. The node's exit must already be a choice.
    AddChoice {
        node: NodeId,
        label: String,
        to: NodeId,
    },
    /// Change an option's label, destination, or both.
    ///
    /// `None` means "leave this field alone", so a caller can relabel without
    /// having to restate the destination it does not want to touch.
    UpdateChoice {
        node: NodeId,
        index: usize,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        to: Option<NodeId>,
    },
    /// Remove an option.
    ///
    /// Refused if it is the last one: a choice exit with no options cannot be
    /// played, and silently converting it to an `End` would change what the story
    /// does without saying so.
    DeleteChoice { node: NodeId, index: usize },

    /// Move an option within its list.
    ///
    /// Order is the order the player reads, so it is part of the writing, not a detail of
    /// storage. Same destination rule as [`EditOp::MoveBeat`]: `to` is the index the option
    /// would occupy before the move, and `to == len` appends.
    MoveChoice {
        node: NodeId,
        from: usize,
        to: usize,
    },

    // --- gameplay outcomes -------------------------------------------------
    /// Map a gameplay outcome key to a node, adding or replacing the mapping.
    ///
    /// `outcome` is whatever the system reports; the editor does not know which keys
    /// are legal, because doing so would mean the editor knew about specific systems.
    SetOutcome {
        node: NodeId,
        outcome: rustory_core::OutcomeKey,
        to: NodeId,
    },
    /// Remove an outcome mapping.
    ///
    /// Allowed to empty the table: a half-written gameplay node is a normal state while
    /// editing, and `problems()` reports it rather than blocking the author from
    /// getting there.
    DeleteOutcome {
        node: NodeId,
        outcome: rustory_core::OutcomeKey,
    },
}

impl EditOp {
    /// Whether this operation can add or remove nodes, beats or options.
    ///
    /// The GUI uses it to decide whether a change might invalidate a selection;
    /// the CLI uses it to label dry runs. Not used for validation — validation is
    /// always the story's own `validate`.
    #[must_use]
    pub fn changes_structure(&self) -> bool {
        matches!(
            self,
            Self::CreateNode { .. }
                | Self::DeleteNode { .. }
                | Self::RenameNode { .. }
                | Self::DuplicateNode { .. }
                | Self::InsertBeat { .. }
                | Self::DeleteBeat { .. }
                | Self::MoveBeat { .. }
                | Self::SetExit { .. }
                | Self::AddChoice { .. }
                | Self::DeleteChoice { .. }
                | Self::MoveChoice { .. }
                | Self::SetOutcome { .. }
                | Self::DeleteOutcome { .. }
        )
    }

    /// Whether this operation should merge into `previous` for undo purposes.
    ///
    /// # Why this exists
    ///
    /// A text field emits an operation per keystroke. Without merging, typing "hello"
    /// leaves five undo steps, and the sixth Ctrl+Z deletes a letter rather than the
    /// sentence — which is worse than no undo, because the user has to count.
    ///
    /// Merging is scoped tightly: same operation kind, same target, and only for edits a
    /// person makes by continuing to type in one place. Two separate visits to the same
    /// field are separate undo steps, because any other operation in between clears the
    /// link — see [`Editor::apply`](crate::Editor::apply).
    #[must_use]
    pub fn coalesces_with(&self, previous: Option<&Self>) -> bool {
        let Some(previous) = previous else {
            return false;
        };

        match (self, previous) {
            (
                Self::UpdateBeat {
                    node: a, index: i, ..
                },
                Self::UpdateBeat {
                    node: b, index: j, ..
                },
            ) => a == b && i == j,
            (
                Self::UpdateChoice {
                    node: a, index: i, ..
                },
                Self::UpdateChoice {
                    node: b, index: j, ..
                },
            ) => a == b && i == j,
            (Self::SetTitle { .. }, Self::SetTitle { .. }) => true,
            (Self::SetAuthor { .. }, Self::SetAuthor { .. }) => true,
            (
                Self::SetOutcome {
                    node: a,
                    outcome: ao,
                    ..
                },
                Self::SetOutcome {
                    node: b,
                    outcome: bo,
                    ..
                },
            ) => a == b && ao == bo,
            // The exit form is one group of controls for one field. Picking `Goto` and
            // then typing an ending name is one act; undo should step behind all of it
            // rather than behind the last letter typed.
            (Self::SetExit { node: a, .. }, Self::SetExit { node: b, .. }) => a == b,
            _ => false,
        }
    }

    /// A short human-readable label, for undo tooltips and CLI logs.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::SetTitle { .. } => "set title".into(),
            Self::SetAuthor { .. } => "set author".into(),
            Self::SetStartNode { node } => format!("start at {node}"),
            Self::CreateNode { id, .. } => format!("create node {id}"),
            Self::DeleteNode { id } => format!("delete node {id}"),
            Self::RenameNode { from, to } => format!("rename {from} to {to}"),
            Self::DuplicateNode { id, as_id } => format!("duplicate {id} as {as_id}"),
            Self::InsertBeat { node, index, .. } => format!("insert beat in {node} at {index}"),
            Self::UpdateBeat { node, index, .. } => format!("edit beat {index} in {node}"),
            Self::DeleteBeat { node, index } => format!("delete beat {index} in {node}"),
            Self::MoveBeat { node, from, to } => format!("move beat {from} to {to} in {node}"),
            Self::SetExit { node, .. } => format!("set exit of {node}"),
            Self::AddChoice { node, .. } => format!("add choice to {node}"),
            Self::UpdateChoice { node, index, .. } => format!("edit choice {index} in {node}"),
            Self::DeleteChoice { node, index } => format!("delete choice {index} in {node}"),
            Self::MoveChoice { node, from, to } => {
                format!("move choice {from} to {to} in {node}")
            }
            Self::SetOutcome { node, outcome, to } => {
                format!("map outcome {outcome} to {to} in {node}")
            }
            Self::DeleteOutcome { node, outcome } => {
                format!("unmap outcome {outcome} in {node}")
            }
        }
    }
}

/// Where a beat was, for locating an error after an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeatLocation {
    pub node: NodeId,
    pub index: usize,
}

/// A beat plus the choice options a node offers, used by the GUI's beat list.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeView {
    pub node: NodeId,
    pub beats: Vec<Beat>,
    pub exit: Exit,
}

impl NodeView {
    #[must_use]
    pub fn choices(&self) -> Option<&[Choice]> {
        match &self.exit {
            Exit::Choose { choices, .. } => Some(choices),
            _ => None,
        }
    }
}
