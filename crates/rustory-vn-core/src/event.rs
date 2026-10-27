//! Facts the narrative publishes for anyone observing it.
//!
//! Events are the only channel from the core outward. A renderer, a save system
//! or a log console subscribes; none of them reaches back in. Keeping that
//! direction fixed is what stops presentation concerns from leaking into the
//! state machine.

use crate::ids::{ActorId, NodeId, OutcomeKey, SystemId};
use crate::story::Slot;

/// Something the narrative did.
/// One thing that happened, in order.
///
/// `PartialEq` and not `Eq`: `SoundPlayed` carries a volume, and floats are not totally
/// ordered. Nothing compares events for identity — they are drained and acted on.
#[derive(Debug, Clone, PartialEq)]
pub enum NarrativeEvent {
    /// Playback moved into a node, before any of its beats ran.
    NodeEntered { node: NodeId },
    /// A line became fully visible (typewriter finished or was skipped).
    LineCompleted { speaker: Option<ActorId> },
    /// An actor appeared or moved.
    ActorShown {
        actor: ActorId,
        pose: String,
        slot: Slot,
    },
    /// An actor left the stage.
    ActorHidden { actor: ActorId },
    /// The background was replaced; `None` clears it.
    BackgroundChanged { image: Option<String> },
    /// The music changed. `None` means it stopped.
    ///
    /// A host turns this into a fade if it wants one; the event says what the story
    /// decided, and `fade_ms` says how long the author asked for.
    MusicChanged { track: Option<String>, fade_ms: u32 },
    /// A one-shot sound was triggered.
    SoundPlayed { clip: String, volume: f32 },
    /// The player is being asked to choose.
    ChoiceOffered { node: NodeId, labels: Vec<String> },
    /// The player chose; `index` refers to the offered list.
    ChoiceTaken { node: NodeId, index: usize },
    /// Control handed to a gameplay system.
    GameplayStarted { node: NodeId, system: SystemId },
    /// The gameplay system reported an outcome and control came back.
    GameplayFinished {
        node: NodeId,
        system: SystemId,
        outcome: OutcomeKey,
    },
    /// The story reached an end node.
    StoryEnded { ending: Option<String> },
}

/// What one [`crate::narrative::Narrative::tick`] changed.
///
/// Renderers use this to decide whether to rebuild text geometry or just advance
/// a reveal count. Reporting it costs nothing and saves a per-frame diff.
///
/// This deliberately says nothing about the cursor: [`crate::Narrative::tick`]
/// only advances a reveal, and moving between beats or nodes is
/// [`crate::Narrative::advance`]'s job. A field for that would always be false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TickReport {
    /// Number of grapheme clusters revealed in the active line after this tick.
    pub revealed: usize,
    /// Whether the active line finished during this tick.
    pub line_completed: bool,
}
