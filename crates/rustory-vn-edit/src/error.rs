//! What can go wrong when editing.
//!
//! Every variant carries the identifiers needed to act on it. "Invalid operation"
//! is useless to an agent; "node `epilogue` is referenced by `prologue` and `choice
//! 2 of ending`" tells it exactly what to fix.

use thiserror::Error;

use rustory_vn_core::{CoreError, NodeId};

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

    #[error("node `{node}`: the gameplay config is not valid RON: {message}")]
    BadConfig { node: NodeId, message: String },

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

impl EditError {
    /// A stable identifier for this failure.
    ///
    /// The message is for a person: it names nodes and explains what to do. A program cannot
    /// branch on prose, and a program that regexes English breaks the moment the wording
    /// improves. These strings are part of the interface — renaming one is a breaking change,
    /// and a test pins that every variant has a distinct one.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoSuchNode(_) => "no_such_node",
            Self::NodeExists(_) => "node_exists",
            Self::BeatOutOfRange { .. } => "beat_out_of_range",
            Self::NoChoices(_) => "no_choices",
            Self::NoGameplay(_) => "no_gameplay",
            Self::NoSuchOutcome { .. } => "no_such_outcome",
            Self::ChoiceOutOfRange { .. } => "choice_out_of_range",
            Self::CannotDeleteStart { .. } => "cannot_delete_start",
            Self::CannotDeleteLast { .. } => "cannot_delete_last_node",
            Self::NodeInUse { .. } => "node_in_use",
            Self::CannotDeleteLastChoice { .. } => "cannot_delete_last_choice",
            Self::EmptyNodeId(_) => "empty_node_id",
            Self::WouldInvalidate(_) => "would_invalidate",
            Self::BadConfig { .. } => "bad_config",
            Self::Io { .. } => "io",
            Self::Parse { .. } => "parse",
            Self::Write { .. } => "write",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variant() -> Vec<EditError> {
        let node = NodeId::from("n");
        vec![
            EditError::NoSuchNode(node.clone()),
            EditError::NodeExists(node.clone()),
            EditError::BeatOutOfRange {
                node: node.clone(),
                index: 0,
                count: 0,
            },
            EditError::NoChoices(node.clone()),
            EditError::NoGameplay(node.clone()),
            EditError::NoSuchOutcome {
                node: node.clone(),
                outcome: "win".into(),
            },
            EditError::ChoiceOutOfRange {
                node: node.clone(),
                index: 0,
                count: 0,
            },
            EditError::CannotDeleteStart { node: node.clone() },
            EditError::CannotDeleteLast { node: node.clone() },
            EditError::NodeInUse {
                node: node.clone(),
                referrers: "start".into(),
            },
            EditError::CannotDeleteLastChoice { node: node.clone() },
            EditError::EmptyNodeId(String::new()),
            EditError::WouldInvalidate(CoreError::MissingStartNode("n".into())),
            EditError::BadConfig {
                node: node.clone(),
                message: "m".into(),
            },
            EditError::Io {
                path: "p".into(),
                message: "m".into(),
            },
            EditError::Parse {
                path: "p".into(),
                line: 1,
                column: 1,
                message: "m".into(),
            },
            EditError::Write {
                path: "p".into(),
                message: "m".into(),
            },
        ]
    }

    #[test]
    fn every_failure_has_its_own_stable_code() {
        let mut seen = std::collections::BTreeSet::new();
        for error in every_variant() {
            let code = error.code();
            assert!(
                seen.insert(code),
                "`{code}` is used by more than one variant, so a caller cannot tell them apart"
            );
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "`{code}` is not a snake_case identifier"
            );
        }
        assert_eq!(seen.len(), 17, "a variant was added without a code");
    }
}
