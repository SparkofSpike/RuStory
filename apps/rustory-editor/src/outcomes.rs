//! Gameplay outcome mappings.
//!
//! A gameplay node hands control to a system, which reports a key. Where that key
//! leads is decided here, in the script — which is what makes the same system reusable
//! with different consequences. Until this existed, mappings could only be changed
//! through `rustory edit --op SetExit`, which meant the GUI could not express something
//! the format allowed. That is exactly the drift the operation layer exists to prevent.
//!
//! # Why these return operations instead of applying them
//!
//! The panel that draws this table already holds an immutable borrow of the document.
//! Taking `&mut EditorApp` here would collide with it, and taking `&mut Vec<EditOp>`
//! would push the decision of *when* to apply into the wrong layer. Returning the
//! operation lets the caller place it wherever its borrows allow.

use rustory_core::{NodeId, OutcomeKey};
use rustory_edit::prelude::*;

/// One mapping, as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub outcome: String,
    pub to: NodeId,
}

/// Read the current mappings for a node.
///
/// Empty when the node is not a gameplay node, which the caller has already checked —
/// returning empty rather than an error keeps this usable from a drawing loop.
#[must_use]
pub fn rows(story: &Story, node: &NodeId) -> Vec<Row> {
    let Some(entry) = story.nodes.get(node) else {
        return Vec::new();
    };
    let Exit::Gameplay { spec } = &entry.exit else {
        return Vec::new();
    };

    spec.outcomes
        .iter()
        .map(|(outcome, to)| Row {
            outcome: outcome.to_string(),
            to: to.clone(),
        })
        .collect()
}

/// Build an operation that maps an outcome to a node.
///
/// The key is free text on purpose: the editor does not know which keys a system
/// reports, and inventing a whitelist would mean the editor knew about specific
/// systems. The runtime already refuses an unmapped key with a named error, so a typo
/// surfaces as a clear failure rather than a silent skip.
///
/// Returns `Err` with a message when the key is unusable, so the caller can show it
/// without duplicating the check.
pub fn set(node: &NodeId, outcome: &str, to: &NodeId) -> Result<EditOp, String> {
    let key = outcome.trim();
    if key.is_empty() {
        return Err("an outcome needs a name".to_owned());
    }
    Ok(EditOp::SetOutcome {
        node: node.clone(),
        outcome: OutcomeKey::from(key),
        to: to.clone(),
    })
}

/// Build an operation that removes a mapping.
#[must_use]
pub fn remove(node: &NodeId, outcome: &str) -> EditOp {
    EditOp::DeleteOutcome {
        node: node.clone(),
        outcome: OutcomeKey::from(outcome),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustory_core::{Exit, GameplaySpec, Node, StoryMeta, SystemId};
    use std::collections::BTreeMap;

    fn gameplay_story() -> Story {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            NodeId::from("play"),
            Node {
                beats: Vec::new(),
                exit: Exit::Gameplay {
                    spec: GameplaySpec {
                        system: SystemId::from("test"),
                        config: None,
                        outcomes: [
                            (OutcomeKey::from("win"), NodeId::from("a")),
                            (OutcomeKey::from("lose"), NodeId::from("b")),
                        ]
                        .into_iter()
                        .collect(),
                    },
                },
            },
        );
        Story {
            meta: StoryMeta::default(),
            start: NodeId::from("play"),
            nodes,
        }
    }

    #[test]
    fn rows_lists_every_mapping_in_key_order() {
        let rows = rows(&gameplay_story(), &NodeId::from("play"));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].outcome, "lose");
        assert_eq!(rows[1].outcome, "win");
    }

    #[test]
    fn rows_of_a_missing_node_is_empty_rather_than_a_panic() {
        // The drawing loop calls this every frame; a selection can go stale between
        // frames when an operation deletes the node underneath it.
        assert!(rows(&gameplay_story(), &NodeId::from("ghost")).is_empty());
    }

    #[test]
    fn an_outcome_key_is_trimmed() {
        let op = set(&NodeId::from("play"), "  win  ", &NodeId::from("a")).expect("valid");
        let EditOp::SetOutcome { outcome, .. } = op else {
            panic!("expected a SetOutcome");
        };
        assert_eq!(outcome.as_str(), "win");
    }

    #[test]
    fn an_empty_key_is_refused_with_a_message_a_person_can_act_on() {
        let err = set(&NodeId::from("play"), "   ", &NodeId::from("a")).expect_err("refused");
        assert!(err.contains("name"), "unhelpful message: {err}");
    }
}
