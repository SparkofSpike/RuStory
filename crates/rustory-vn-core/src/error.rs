//! Errors raised by the narrative core.

use thiserror::Error;

/// What can go wrong when building or driving a story.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("story declares start node `{0}`, which does not exist")]
    MissingStartNode(String),

    #[error("this story has no node `{0}`")]
    NoSuchNode(String),

    #[error("node `{node}` refers to node `{target}`, which does not exist")]
    DanglingNodeRef { node: String, target: String },

    #[error("node `{node}` hands control to gameplay system `{system}`, which is not registered")]
    UnknownGameplaySystem { node: String, system: String },

    #[error("node `{node}` offers choice {index}, but it offers only {count} choices")]
    ChoiceOutOfRange {
        node: String,
        index: usize,
        count: usize,
    },

    #[error("node `{node}` is not waiting for a choice")]
    NotAwaitingChoice { node: String },

    #[error("node `{node}` is not waiting for a gameplay verdict")]
    NotAwaitingGameplay { node: String },

    #[error(
        "gameplay system `{system}` reported outcome `{outcome}`, which node `{node}` does not handle"
    )]
    UnmappedOutcome {
        node: String,
        system: String,
        outcome: String,
    },

    #[error("gameplay system `{system}` failed: {message}")]
    GameplayFailed { system: String, message: String },

    #[error("gameplay system `{system}` could not save its state: {message}")]
    GameplaySaveFailed { system: String, message: String },

    #[error("gameplay system `{system}` could not restore its state: {message}")]
    GameplayRestoreFailed { system: String, message: String },

    #[error(
        "save format version {found} is not supported (this build writes and reads version {expected})"
    )]
    SaveVersionMismatch { found: u32, expected: u32 },

    #[error(
        "save was written for a different story (checksum {found:#x}, this story is {expected:#x})"
    )]
    SaveStoryMismatch { found: u64, expected: u64 },

    #[error("save refers to node `{node}`, which this story no longer has")]
    SaveNodeMissing { node: String },

    #[error(
        "save places the cursor at beat {index} of node `{node}`, which has only {count} beats"
    )]
    SaveBeatOutOfRange {
        node: String,
        index: usize,
        count: usize,
    },

    #[error(
        "save is suspended inside gameplay system `{saved}`, but node `{node}` now hands over to `{current}`"
    )]
    SaveGameplayMismatch {
        node: String,
        saved: String,
        current: String,
    },

    #[error("node `{node}` ran more than {limit} beats without waiting for the player")]
    EndlessFlow { node: String, limit: usize },
}
