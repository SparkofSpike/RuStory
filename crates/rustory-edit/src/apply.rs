//! Applying one operation to a story.
//!
//! Pure functions over [`Story`]: no I/O, no history, no validation. The editor
//! layers those on. Keeping this layer pure is what makes every operation testable
//! in three lines.

use std::collections::BTreeMap;

use rustory_core::{Exit, Node, NodeId, Story};

use crate::error::EditError;
use crate::op::EditOp;

/// Apply `op` to `story` in place.
///
/// Structural preconditions are checked here — indices in range, nodes present,
/// deletions safe. Global validity is *not*: the editor validates afterwards, so
/// there is exactly one place that decides what "valid" means.
pub(crate) fn apply(story: &mut Story, op: &EditOp) -> Result<(), EditError> {
    match op {
        EditOp::SetTitle { title } => {
            story.meta.title = title.clone();
        }
        EditOp::SetAuthor { author } => {
            story.meta.author = author.clone();
        }
        EditOp::SetStartNode { node } => {
            if !story.nodes.contains_key(node) {
                return Err(EditError::NoSuchNode(node.clone()));
            }
            story.start = node.clone();
        }

        EditOp::CreateNode { id, at: _ } => {
            if id.as_str().trim().is_empty() {
                return Err(EditError::EmptyNodeId(id.to_string()));
            }
            if story.nodes.contains_key(id) {
                return Err(EditError::NodeExists(id.clone()));
            }
            story.nodes.insert(
                id.clone(),
                Node {
                    beats: Vec::new(),
                    exit: Exit::End { ending: None },
                },
            );
        }

        EditOp::DeleteNode { id } => {
            let node = story
                .nodes
                .get(id)
                .ok_or_else(|| EditError::NoSuchNode(id.clone()))?;
            let _ = node;

            if story.nodes.len() == 1 {
                return Err(EditError::CannotDeleteLast { node: id.clone() });
            }
            if &story.start == id {
                return Err(EditError::CannotDeleteStart { node: id.clone() });
            }

            let referrers = referrers_to(story, id);
            if !referrers.is_empty() {
                return Err(EditError::NodeInUse {
                    node: id.clone(),
                    referrers: describe_referrers(&referrers),
                });
            }

            story.nodes.remove(id);
        }

        EditOp::RenameNode { from, to } => {
            if from == to {
                return Ok(());
            }
            if !story.nodes.contains_key(from) {
                return Err(EditError::NoSuchNode(from.clone()));
            }
            if to.as_str().trim().is_empty() {
                return Err(EditError::EmptyNodeId(to.to_string()));
            }
            if story.nodes.contains_key(to) {
                return Err(EditError::NodeExists(to.clone()));
            }

            // Move the node body, then rewrite every pointer. Doing it in this order
            // means a mid-way failure is impossible: there are no fallible steps
            // after the existence checks.
            let mut nodes = std::mem::take(&mut story.nodes);
            let mut renamed = BTreeMap::new();
            for (id, node) in std::mem::take(&mut nodes) {
                let key = if &id == from { to.clone() } else { id };
                renamed.insert(key, node);
            }
            story.nodes = renamed;

            rewrite_references(story, from, to);
        }

        EditOp::DuplicateNode { id, as_id } => {
            let source = story
                .nodes
                .get(id)
                .ok_or_else(|| EditError::NoSuchNode(id.clone()))?
                .clone();
            if as_id.as_str().trim().is_empty() {
                return Err(EditError::EmptyNodeId(as_id.to_string()));
            }
            if story.nodes.contains_key(as_id) {
                return Err(EditError::NodeExists(as_id.clone()));
            }
            story.nodes.insert(as_id.clone(), source);
        }

        EditOp::InsertBeat { node, index, beat } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            if *index > node_ref.beats.len() {
                return Err(EditError::BeatOutOfRange {
                    node: node.clone(),
                    index: *index,
                    count: node_ref.beats.len(),
                });
            }
            node_ref.beats.insert(*index, beat.clone());
        }

        EditOp::UpdateBeat { node, index, beat } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            let count = node_ref.beats.len();
            let slot = node_ref
                .beats
                .get_mut(*index)
                .ok_or_else(|| EditError::BeatOutOfRange {
                    node: node.clone(),
                    index: *index,
                    count,
                })?;
            *slot = beat.clone();
        }

        EditOp::DeleteBeat { node, index } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            if *index >= node_ref.beats.len() {
                return Err(EditError::BeatOutOfRange {
                    node: node.clone(),
                    index: *index,
                    count: node_ref.beats.len(),
                });
            }
            node_ref.beats.remove(*index);
        }

        EditOp::MoveBeat { node, from, to } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            let count = node_ref.beats.len();
            if *from >= count {
                return Err(EditError::BeatOutOfRange {
                    node: node.clone(),
                    index: *from,
                    count,
                });
            }
            // `to == count` is a legal destination: it means "move to the end".
            if *to > count {
                return Err(EditError::BeatOutOfRange {
                    node: node.clone(),
                    index: *to,
                    count,
                });
            }
            let beat = node_ref.beats.remove(*from);
            // Removing earlier shifts the insertion point left.
            let target = if *to > *from { *to - 1 } else { *to };
            node_ref.beats.insert(target, beat);
        }

        EditOp::SetExit { node, exit } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            node_ref.exit = exit.clone();
        }

        EditOp::AddChoice { node, label, to } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            match &mut node_ref.exit {
                Exit::Choose { choices, .. } => choices.push(rustory_core::Choice {
                    label: label.clone(),
                    to: to.clone(),
                }),
                _ => return Err(EditError::NoChoices(node.clone())),
            }
        }

        EditOp::UpdateChoice {
            node,
            index,
            label,
            to,
        } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            let Exit::Choose { choices, .. } = &mut node_ref.exit else {
                return Err(EditError::NoChoices(node.clone()));
            };
            let count = choices.len();
            let choice = choices
                .get_mut(*index)
                .ok_or_else(|| EditError::ChoiceOutOfRange {
                    node: node.clone(),
                    index: *index,
                    count,
                })?;
            if let Some(label) = label {
                choice.label.clone_from(label);
            }
            if let Some(to) = to {
                choice.to.clone_from(to);
            }
        }

        EditOp::DeleteChoice { node, index } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            let Exit::Choose { choices, .. } = &mut node_ref.exit else {
                return Err(EditError::NoChoices(node.clone()));
            };
            if *index >= choices.len() {
                return Err(EditError::ChoiceOutOfRange {
                    node: node.clone(),
                    index: *index,
                    count: choices.len(),
                });
            }
            if choices.len() == 1 {
                return Err(EditError::CannotDeleteLastChoice { node: node.clone() });
            }
            choices.remove(*index);
        }

        EditOp::MoveChoice { node, from, to } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            let Exit::Choose { choices, .. } = &mut node_ref.exit else {
                return Err(EditError::NoChoices(node.clone()));
            };
            let count = choices.len();
            if *from >= count {
                return Err(EditError::ChoiceOutOfRange {
                    node: node.clone(),
                    index: *from,
                    count,
                });
            }
            // `to == count` is a legal destination: it means "move to the end".
            if *to > count {
                return Err(EditError::ChoiceOutOfRange {
                    node: node.clone(),
                    index: *to,
                    count,
                });
            }
            let choice = choices.remove(*from);
            // Removing earlier shifts the insertion point left, exactly as in `MoveBeat`.
            let target = if *to > *from { *to - 1 } else { *to };
            choices.insert(target, choice);
        }

        EditOp::SetGameplayConfig { node, config } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            let Exit::Gameplay { spec } = &mut node_ref.exit else {
                return Err(EditError::NoGameplay(node.clone()));
            };
            // Parsed before assigning, so a typo leaves the previous config in place rather
            // than replacing a working scene with an empty one.
            spec.config =
                crate::config::parse(config.as_deref().unwrap_or("")).map_err(|message| {
                    EditError::BadConfig {
                        node: node.clone(),
                        message,
                    }
                })?;
        }

        EditOp::SetOutcome { node, outcome, to } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            let Exit::Gameplay { spec } = &mut node_ref.exit else {
                return Err(EditError::NoGameplay(node.clone()));
            };
            spec.outcomes.insert(outcome.clone(), to.clone());
        }

        EditOp::DeleteOutcome { node, outcome } => {
            let node_ref = story
                .nodes
                .get_mut(node)
                .ok_or_else(|| EditError::NoSuchNode(node.clone()))?;
            let Exit::Gameplay { spec } = &mut node_ref.exit else {
                return Err(EditError::NoGameplay(node.clone()));
            };
            if spec.outcomes.remove(outcome).is_none() {
                return Err(EditError::NoSuchOutcome {
                    node: node.clone(),
                    outcome: outcome.to_string(),
                });
            }
        }
    }

    Ok(())
}

/// One place that points at a node.
///
/// Public because a front-end showing "what would break if I deleted this" is more useful
/// before the attempt than after the refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    Start,
    Goto {
        node: NodeId,
    },
    Choice {
        node: NodeId,
        index: usize,
        label: String,
    },
    Outcome {
        node: NodeId,
        system: String,
        outcome: String,
    },
}

/// Every reference to `target`, in a stable order.
///
/// Used by deletion (to explain why it is refused) and by rename (to rewrite). One
/// function for both means the two can never disagree about what counts as a
/// reference.
pub(crate) fn referrers_to(story: &Story, target: &NodeId) -> Vec<Reference> {
    let mut found = Vec::new();

    if &story.start == target {
        found.push(Reference::Start);
    }

    for (id, node) in &story.nodes {
        match &node.exit {
            Exit::End { .. } => {}
            Exit::Goto { to } if to == target => {
                found.push(Reference::Goto { node: id.clone() });
            }
            Exit::Goto { .. } => {}
            Exit::Choose { choices, .. } => {
                for (index, choice) in choices.iter().enumerate() {
                    if &choice.to == target {
                        found.push(Reference::Choice {
                            node: id.clone(),
                            index,
                            label: choice.label.clone(),
                        });
                    }
                }
            }
            Exit::Gameplay { spec } => {
                for (outcome, to) in &spec.outcomes {
                    if to == target {
                        found.push(Reference::Outcome {
                            node: id.clone(),
                            system: spec.system.to_string(),
                            outcome: outcome.to_string(),
                        });
                    }
                }
            }
        }
    }

    found
}

fn describe_referrers(referrers: &[Reference]) -> String {
    referrers
        .iter()
        .map(Reference::describe)
        .collect::<Vec<_>>()
        .join(", ")
}

impl Reference {
    /// The node this reference comes from, when it comes from one.
    ///
    /// `Start` is a reference from the story itself, so there is nowhere to jump to.
    #[must_use]
    pub fn source(&self) -> Option<&NodeId> {
        match self {
            Self::Start => None,
            Self::Goto { node } | Self::Choice { node, .. } | Self::Outcome { node, .. } => {
                Some(node)
            }
        }
    }

    /// One line a person can read, used in errors and in the editor.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Start => "the story's start node".to_owned(),
            Self::Goto { node } => format!("a goto in `{node}`"),
            Self::Choice { node, index, label } => {
                format!("choice {} (`{label}`) of `{node}`", index + 1)
            }
            Self::Outcome {
                node,
                system,
                outcome,
            } => format!("outcome `{outcome}` of `{system}` in `{node}`"),
        }
    }
}

/// Point every reference to `from` at `to` instead.
fn rewrite_references(story: &mut Story, from: &NodeId, to: &NodeId) {
    if &story.start == from {
        story.start = to.clone();
    }

    for node in story.nodes.values_mut() {
        match &mut node.exit {
            Exit::End { .. } => {}
            Exit::Goto { to: target } => {
                if target == from {
                    *target = to.clone();
                }
            }
            Exit::Choose { choices, .. } => {
                for choice in choices {
                    if &choice.to == from {
                        choice.to = to.clone();
                    }
                }
            }
            Exit::Gameplay { spec } => {
                for target in spec.outcomes.values_mut() {
                    if target == from {
                        *target = to.clone();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustory_core::{Choice, GameplaySpec, Node, StoryMeta, SystemId};
    use std::collections::BTreeMap;

    fn node(exit: Exit) -> Node {
        Node {
            beats: Vec::new(),
            exit,
        }
    }

    /// `a` goes to `b`; `b` offers a choice to `c` and a gameplay handover whose
    /// outcomes also point at `c`; `c` ends. `a` is the start node.
    fn story() -> Story {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            NodeId::from("a"),
            node(Exit::Goto {
                to: NodeId::from("b"),
            }),
        );
        nodes.insert(
            NodeId::from("b"),
            node(Exit::Choose {
                prompt: None,
                choices: vec![Choice {
                    label: "onward".into(),
                    to: NodeId::from("c"),
                }],
            }),
        );
        let mut outcomes = BTreeMap::new();
        outcomes.insert(rustory_core::OutcomeKey::from("win"), NodeId::from("c"));
        nodes.insert(
            NodeId::from("c"),
            node(Exit::Gameplay {
                spec: GameplaySpec {
                    system: SystemId::from("timer.timed_choice"),
                    config: None,
                    outcomes,
                },
            }),
        );
        Story {
            meta: StoryMeta::default(),
            start: NodeId::from("a"),
            nodes,
        }
    }

    #[test]
    fn every_kind_of_reference_is_found() {
        let story = story();

        assert_eq!(
            referrers_to(&story, &NodeId::from("b")),
            vec![Reference::Goto {
                node: NodeId::from("a")
            }]
        );
        assert_eq!(
            referrers_to(&story, &NodeId::from("c")),
            vec![
                Reference::Choice {
                    node: NodeId::from("b"),
                    index: 0,
                    label: "onward".into(),
                },
                Reference::Outcome {
                    node: NodeId::from("c"),
                    system: "timer.timed_choice".into(),
                    outcome: "win".into(),
                },
            ]
        );
        assert_eq!(
            referrers_to(&story, &NodeId::from("a")),
            vec![Reference::Start]
        );
    }

    #[test]
    fn an_unreferenced_node_has_no_referrers_and_a_description_names_its_source() {
        let story = story();
        assert!(referrers_to(&story, &NodeId::from("nowhere")).is_empty());

        let choice = Reference::Choice {
            node: NodeId::from("b"),
            index: 2,
            label: "third".into(),
        };
        assert_eq!(choice.source(), Some(&NodeId::from("b")));
        assert_eq!(choice.describe(), "choice 3 (`third`) of `b`");
        assert_eq!(Reference::Start.source(), None);
    }
}
