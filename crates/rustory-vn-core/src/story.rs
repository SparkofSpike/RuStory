//! The story domain model.
//!
//! This is what a script compiles down to: a directed graph of [`Node`]s, each
//! holding an ordered list of [`Beat`]s and one [`Exit`]. The model lives in the
//! core rather than in the script crate because the state machine drives it
//! directly; the script crate's job is only to *produce* one of these from text.
//!
//! Two design points worth knowing before editing:
//!
//! * **Beats are commands, not dialogue.** `Say` renders text, `ShowActor` moves
//!   a sprite. Keeping them in one list means an author writes the scene in the
//!   order it happens, and the state machine needs only one cursor.
//! * **The core does not know any gameplay's config shape.** [`GameplaySpec`]
//!   carries an opaque [`ron::Value`] that only the registered system parses.
//!   That is what makes "add a gameplay system without touching the core" true
//!   rather than aspirational.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::{ActorId, NodeId, OutcomeKey, SystemId};

/// Where an actor stands on stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Slot {
    Left,
    Center,
    Right,
}

/// One command in a node's sequence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Beat {
    /// Speak a line. `speaker` is `None` for narration.
    Say {
        speaker: Option<ActorId>,
        text: String,
    },
    /// Put an actor on stage, or move one already there.
    ShowActor {
        actor: ActorId,
        pose: String,
        slot: Slot,
    },
    /// Take an actor off stage. Removing an absent actor is a no-op, so scripts
    /// do not need to track stage state themselves.
    HideActor { actor: ActorId },
    /// Replace the background image. `None` clears it.
    SetBackground { image: Option<String> },
    /// Start, change or stop the music. `None` silences it.
    ///
    /// `fade_ms` is how long the change takes; zero or absent means instantly. A track name
    /// is a file stem that the host resolves, exactly like a background image: the core has
    /// no opinion about file formats or where audio lives.
    Music {
        track: Option<String>,
        #[serde(default)]
        fade_ms: Option<u32>,
    },
    /// Play a one-shot sound. Not remembered: a sound effect is an event, not a state, so
    /// loading a save in the middle of a line does not replay a door slam.
    Sound {
        clip: String,
        #[serde(default)]
        volume: Option<f32>,
    },
}

/// A branch the player can pick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub label: String,
    pub to: NodeId,
}

/// Hand control to a gameplay system, then branch on what it reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameplaySpec {
    /// Which registered system takes over.
    pub system: SystemId,
    /// Configuration handed to that system verbatim. The core never inspects it.
    #[serde(default)]
    pub config: Option<ron::Value>,
    /// Where to go for each outcome the system may report. A system reporting a
    /// key absent from this table is an error, not a silent fallthrough —
    /// unhandled outcomes hide bugs.
    #[serde(default)]
    pub outcomes: BTreeMap<OutcomeKey, NodeId>,
}

/// What happens when a node's beats run out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Exit {
    /// The story stops here.
    End {
        #[serde(default)]
        ending: Option<String>,
    },
    /// Continue at another node.
    Goto { to: NodeId },
    /// Stop and let the player pick.
    Choose {
        #[serde(default)]
        prompt: Option<String>,
        choices: Vec<Choice>,
    },
    /// Suspend the narrative and let a gameplay system drive.
    Gameplay { spec: GameplaySpec },
}

/// A scene: beats to play, then one exit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    #[serde(default)]
    pub beats: Vec<Beat>,
    pub exit: Exit,
}

/// Story-level metadata. Carried through for tooling; the state machine ignores it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StoryMeta {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
}

/// The whole story graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Story {
    #[serde(default)]
    pub meta: StoryMeta,
    /// Entry point. Must exist in `nodes`.
    pub start: NodeId,
    pub nodes: BTreeMap<NodeId, Node>,
}

impl Story {
    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Referential integrity, checked before the story is ever played.
    ///
    /// Every reference is verified here rather than at the point of use, so a
    /// broken script fails at load with the offending node named — not three
    /// clicks into the scene the player is watching.
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        use crate::error::CoreError;

        if !self.nodes.contains_key(&self.start) {
            return Err(CoreError::MissingStartNode(self.start.to_string()));
        }

        for (id, node) in &self.nodes {
            let dangling = |target: &NodeId| {
                if self.nodes.contains_key(target) {
                    None
                } else {
                    Some(CoreError::DanglingNodeRef {
                        node: id.to_string(),
                        target: target.to_string(),
                    })
                }
            };

            match &node.exit {
                Exit::End { .. } => {}
                Exit::Goto { to } => {
                    if let Some(err) = dangling(to) {
                        return Err(err);
                    }
                }
                Exit::Choose { choices, .. } => {
                    for choice in choices {
                        if let Some(err) = dangling(&choice.to) {
                            return Err(err);
                        }
                    }
                }
                Exit::Gameplay { spec } => {
                    for target in spec.outcomes.values() {
                        if let Some(err) = dangling(target) {
                            return Err(err);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Total number of beats, used by tests and progress reporting.
    #[must_use]
    pub fn beat_count(&self) -> usize {
        self.nodes.values().map(|node| node.beats.len()).sum()
    }

    /// A stable fingerprint of the story's content.
    ///
    /// Saves record this so loading an old save against an edited script fails
    /// loudly instead of resuming at a node whose beats have shifted under the
    /// player. FNV-1a over the RON form: `BTreeMap` iterates in key order, so the
    /// same story always hashes the same, across processes and compilations.
    ///
    /// This is a change detector, not a security primitive. Nothing here defends
    /// against a deliberately crafted collision; a save file is the player's own
    /// data.
    #[must_use]
    pub fn checksum(&self) -> u64 {
        // RON can only fail on types it cannot represent, and this type is plain
        // data. Falling back to zero would silently disable the check, so a
        // failure is surfaced as a checksum that can never match a real one.
        let Ok(encoded) = ron::to_string(self) else {
            return u64::MAX;
        };

        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in encoded.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}
