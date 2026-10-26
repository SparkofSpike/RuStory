//! What can go wrong when editing.
//!
//! Every variant carries the identifiers needed to act on it. "Invalid operation"
//! is useless to an agent; "node `epilogue` is referenced by `prologue` and `choice
//! 2 of ending`" tells it exactly what to fix.

use thiserror::Error;

use rustory_core::{CoreError, NodeId};

#[derive(Debug, Error)]
pub enum EditError {
    #[error("node `{0}` does not exist")]
    NoSuchNode(NodeId),

    #[error("node `{0}` already exists")]
    NodeExists(NodeId),

    #[error("beat index {index} is out of range for node `{node}` ({count} beats)")]
    BeatOutOfRange {
        node: NodeId,
        index: usize,
        count: usize,
    },

    #[error("node `{0}` does not offer choices, so it has no options to edit")]
    NoChoices(NodeId),

    #[error("node `{0}` does not hand over to a gameplay system, so it has no outcomes")]
    NoGameplay(NodeId),

    #[error("node `{node}` has no mapping for outcome `{outcome}`")]
    NoSuchOutcome { node: NodeId, outcome: String },

    #[error("choice index {index} is out of range for node `{node}` ({count} options)")]
    ChoiceOutOfRange {
        node: NodeId,
        index: usize,
        count: usize,
    },

    #[error("cannot delete node `{node}`: it is the story's start node")]
    CannotDeleteStart { node: NodeId },

    #[error("cannot delete node `{node}`: it is the only node in the story")]
    CannotDeleteLast { node: NodeId },

    #[error(
        "cannot delete node `{node}`: still referenced by {referrers}. \
         Retarget or delete those first, or rename the node instead."
    )]
    NodeInUse { node: NodeId, referrers: String },

    #[error(
        "cannot delete the last choice of node `{node}`: a choice exit needs at least one option"
    )]
    CannotDeleteLastChoice { node: NodeId },

    #[error("node id `{0}` is empty")]
    EmptyNodeId(String),

    #[error("the edit would leave the story invalid: {0}")]
    WouldInvalidate(#[from] CoreError),

    #[error("cannot read {path}: {message}")]
    Io { path: String, message: String },

    #[error("cannot parse {path}:{line}:{column}: {message}")]
    Parse {
        path: String,
        line: usize,
        column: usize,
        message: String,
    },

    #[error("cannot write {path}: {message}")]
    Write { path: String, message: String },
}
