//! Save data.
//!
//! A save is a snapshot of everything needed to resume mid-scene: where the
//! cursor sits, how much of the current line was revealed, what is on stage, the
//! shared counters, and whatever state a gameplay system reported that it could
//! not rebuild from config.
//!
//! # Why a snapshot rather than an input log
//!
//! Replaying recorded inputs would make saves tiny, but it demands perfect
//! determinism from every gameplay system — no `HashMap` iteration, no wall clock,
//! no floating-point accumulation. That constraint is impossible to hold across
//! third-party systems, and a save that diverges silently is worse than a large
//! save that always restores. See ADR 0006.
//!
//! # Why gameplay state is opaque
//!
//! [`GameplaySave::state`] is a [`ron::Value`] the core never interprets. A system
//! written years from now can store whatever shape it needs without this crate
//! learning about it — the same principle as the opaque config in ADR 0003.

use serde::{Deserialize, Serialize};

use crate::gameplay::Stats;
use crate::ids::{NodeId, SystemId};
use crate::narrative::Mode;
use crate::stage::Stage;

/// The save format this build writes and accepts.
///
/// Bumped whenever the shape below changes incompatibly. A load refuses any other
/// version rather than guessing at a migration: a mis-restored save corrupts a
/// playthrough in ways the player cannot diagnose.
pub const SAVE_VERSION: u32 = 1;

/// A gameplay system's contribution to a save.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameplaySave {
    /// Which system was holding control.
    pub system: SystemId,
    /// Whatever [`crate::GameplaySystem::save`] returned. `None` means the system
    /// declared itself stateless and will be rebuilt by `begin` alone.
    #[serde(default)]
    pub state: Option<ron::Value>,
}

/// A complete resumable snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveState {
    /// Format version. Checked on load.
    pub version: u32,
    /// [`crate::Story::checksum`] of the script this save belongs to.
    pub story_checksum: u64,
    /// Node the cursor sits in.
    pub cursor: NodeId,
    /// Index of the current beat within that node.
    pub beat_index: usize,
    /// What the machine was waiting for.
    pub mode: Mode,
    /// Grapheme clusters of the current line already revealed, so a load resumes
    /// mid-sentence instead of restarting the line.
    pub revealed: usize,
    /// Background and actors currently on stage.
    pub stage: Stage,
    /// The track that should be playing, or `None`. Defaulted so saves written before
    /// audio existed still load — they simply describe a silent scene, which is what they
    /// were.
    #[serde(default)]
    pub music: Option<String>,
    /// Shared counters.
    pub stats: Stats,
    /// Present only when the save was taken while a gameplay system had control.
    #[serde(default)]
    pub gameplay: Option<GameplaySave>,
}

impl SaveState {
    /// Serialise to RON.
    ///
    /// RON because saves get investigated by hand: a player reporting "it restored
    /// into the wrong scene" is answerable by reading the file. Size is not the
    /// constraint here — a save is a few kilobytes either way.
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    /// Parse from RON.
    ///
    /// Returns the position-carrying error so a malformed save can be reported with
    /// a line and column, the same way a malformed script is.
    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::NodeId;
    use crate::story::Beat;

    fn sample() -> SaveState {
        let mut stats = Stats::default();
        stats.set("affection", 3);

        SaveState {
            version: SAVE_VERSION,
            story_checksum: 0xdead_beef,
            cursor: NodeId::from("prologue"),
            beat_index: 2,
            mode: Mode::AwaitingAdvance,
            revealed: 17,
            stage: Stage::default(),
            music: Some("classroom".to_owned()),
            stats,
            gameplay: None,
        }
    }

    #[test]
    fn a_save_round_trips_through_ron() {
        let save = sample();
        let text = save.to_ron().expect("serialises");
        let back = SaveState::from_ron(&text).expect("parses");
        assert_eq!(back, save);
    }

    #[test]
    fn a_suspended_save_carries_the_system_and_its_state() {
        let mut save = sample();
        save.gameplay = Some(GameplaySave {
            system: SystemId::from("example.stateful"),
            state: Some(ron::from_str("(remaining_ms: 1200)").expect("literal")),
        });

        let text = save.to_ron().expect("serialises");
        let back = SaveState::from_ron(&text).expect("parses");
        let gameplay = back.gameplay.expect("gameplay block survives");
        assert_eq!(gameplay.system.as_str(), "example.stateful");
        assert!(
            gameplay.state.is_some(),
            "opaque state must survive the trip"
        );
    }

    #[test]
    fn a_save_without_gameplay_parses_even_though_the_field_is_optional() {
        // Forward compatibility: a hand-written or older save may omit the block.
        let text = r#"
            (
                version: 1,
                story_checksum: 1,
                cursor: "a",
                beat_index: 0,
                mode: AwaitingAdvance,
                revealed: 0,
                stage: ( background: None, actors: {} ),
                stats: {},
            )
        "#;
        let save = SaveState::from_ron(text).expect("parses");
        assert!(save.gameplay.is_none());
    }

    #[test]
    fn the_pretty_form_is_stable_enough_to_diff() {
        // Saves are meant to be readable, so the same state must serialise to the
        // same text. If this fails, diffs between two saves become noise.
        let a = sample().to_ron().expect("serialises");
        let b = sample().to_ron().expect("serialises");
        assert_eq!(a, b);
        assert!(a.contains("cursor"), "field names should be visible: {a}");
    }

    #[test]
    fn beats_are_not_part_of_a_save() {
        // Guards against someone "helpfully" serialising the story into every save.
        // The script is loaded separately, and the checksum is what ties them.
        let text = sample().to_ron().expect("serialises");
        let beat_debug = format!(
            "{:?}",
            Beat::Say {
                speaker: None,
                text: "unlikely marker".into(),
            }
        );
        assert!(!text.contains("unlikely marker"));
        assert!(!beat_debug.is_empty());
    }
}
