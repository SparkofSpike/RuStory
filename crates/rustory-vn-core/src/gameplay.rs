//! The gameplay contract.
//!
//! # Why this shape
//!
//! The goal is that adding a gameplay system never edits the core. Two things
//! make that real:
//!
//! 1. [`GameplaySpec`](crate::story::GameplaySpec) carries its configuration as
//!    an opaque [`ron::Value`]. The core stores and forwards it but never parses
//!    it, so no new gameplay type forces a new core enum variant.
//! 2. A system reports an [`OutcomeKey`], and the *script* decides where each key
//!    leads. The core therefore does not know what "win" or "lose" mean.
//!
//! A system is a trait object in a registry, created by id. That is deliberate
//! type erasure at exactly one seam: everything on either side of it is strongly
//! typed, and the erased surface is three methods wide.
//!
//! # What a system may touch
//!
//! Its own fields, [`Stats`], and a read-only [`NarrativeSnapshot`] of where the story is.
//! It cannot see the stage, the story graph or the renderer, and it cannot write the story:
//! the only way a gameplay affects what happens next is by reporting an [`OutcomeKey`]. If a
//! system needs new shared state, that is a deliberate widening of this contract and belongs
//! in an ADR, not in a `&mut World` escape hatch.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::ids::{NodeId, OutcomeKey, SystemId};

/// Named integer values shared between the narrative and gameplay systems.
///
/// This is intentionally the narrowest useful surface: counters, scores, flags.
/// Keyed by string because different stories invent different attributes, but the
/// value type is fixed — a bag of `Any` here would be the exact dynamic-shape
/// improvisation this project exists to avoid.
///
/// Serialised as a plain map so a save file stays readable and hand-editable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Stats {
    values: BTreeMap<String, i32>,
}

impl Stats {
    #[must_use]
    pub fn get(&self, key: &str) -> i32 {
        self.values.get(key).copied().unwrap_or(0)
    }

    pub fn set(&mut self, key: impl Into<String>, value: i32) {
        self.values.insert(key.into(), value);
    }

    pub fn add(&mut self, key: impl Into<String>, delta: i32) {
        let entry = self.values.entry(key.into()).or_insert(0);
        *entry = entry.saturating_add(delta);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, i32)> {
        self.values.iter().map(|(k, v)| (k.as_str(), *v))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// One frame's worth of input, already translated by the application layer.
///
/// Gameplay systems must not care whether input came from a mouse, a key or a
/// network peer, so they receive this instead of platform events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameplayInput {
    /// The player clicked, tapped or pressed confirm.
    Confirm,
    /// The player chose option `index` from whatever the system presented.
    Select { index: usize },
}

/// What a gameplay system may know about the story around it.
///
/// Read-only and owned, copied at the handover rather than borrowed from the narrative: a
/// system can see where it is but cannot reach back in, so adding a gameplay cannot change what
/// happens outside its own handover. That is the difference between this and the `&mut World`
/// escape hatch ADR 0004 refused.
///
/// This exists because a system whose rules depend on what the player has read — a debate that
/// escalates with each round, a puzzle that reuses a clue from three scenes ago — could only
/// hard-code the answer in its config before. Widening the contract is the honest fix; the
/// alternative was every such system inventing its own half-working counter in [`Stats`].
///
/// No `Default`: a snapshot is produced by a narrative, and an empty one would have to invent
/// a node, which is the kind of placeholder that ends up compared against by accident.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NarrativeSnapshot {
    /// The node that handed control over.
    pub node: NodeId,
    /// Zero-based index of the beat the handover happened at.
    ///
    /// Usually the node's last beat, since a handover is an exit — but a node can also stop
    /// early, and a system drawing a progress bar wants the real number.
    pub beat_index: usize,
    /// How many beats that node has.
    pub beat_count: usize,
    /// Nodes entered so far, oldest first, repeats included.
    ///
    /// Repeats are kept on purpose: "is this the second time we are here" is the question a
    /// boss fight asks, and a set would answer it wrongly. A caller that only wants "have we
    /// ever been here" checks [`NarrativeSnapshot::has_visited`].
    pub visited: Vec<NodeId>,
    /// Branches the player has taken, oldest first.
    pub choices: Vec<ChoiceMade>,
}

impl NarrativeSnapshot {
    /// How many times `node` has been entered, this visit included.
    #[must_use]
    pub fn visits(&self, node: &str) -> usize {
        self.visited
            .iter()
            .filter(|seen| seen.as_str() == node)
            .count()
    }

    /// Whether `node` has been entered at least once.
    #[must_use]
    pub fn has_visited(&self, node: &str) -> bool {
        self.visited.iter().any(|seen| seen.as_str() == node)
    }
}

/// One branch the player took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceMade {
    /// The node that offered it.
    pub node: NodeId,
    /// Which option, zero-based, as the script listed it.
    pub index: usize,
}

/// What a gameplay system is allowed to see and change.
pub struct GameplayContext<'a> {
    /// Milliseconds since the previous step. Wall-clock stays in the app layer.
    pub dt_ms: u32,
    /// Inputs collected since the previous step.
    pub inputs: &'a [GameplayInput],
    /// Shared counters. The only narrative-adjacent state gameplay may change.
    pub stats: &'a mut Stats,
    /// Where the story is, frozen at the handover.
    ///
    /// Read-only: nothing here can be written back, which is what makes "a gameplay cannot
    /// affect the story except through its outcome" a fact rather than a convention.
    pub narrative: &'a NarrativeSnapshot,
}

/// A pluggable gameplay system.
///
/// Implementors live in their own crate. Adding one means registering it; the
/// narrative core compiles unchanged.
pub trait GameplaySystem {
    /// The id scripts use to invoke this system.
    fn id(&self) -> SystemId;

    /// Called once when the script hands control over, with that node's config and the
    /// story's position.
    ///
    /// Parsing happens here rather than in the loader so the core stays ignorant of the
    /// concrete config type. Returning an error aborts the handover and surfaces to the
    /// caller; it must not leave half-initialized state behind.
    ///
    /// The snapshot is passed here as well as to [`GameplaySystem::step`] because a system
    /// that varies its content by context — a quiz that picks harder questions the second
    /// time through — has to choose before the first frame, not after it.
    fn begin(
        &mut self,
        config: Option<&ron::Value>,
        narrative: &NarrativeSnapshot,
    ) -> Result<(), CoreError>;

    /// Advance by `dt_ms`. Return `Some(outcome)` when the system is finished,
    /// `None` to keep control.
    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey>;

    /// The narrative is aborting this system (story closed, save loaded past it).
    /// Must reset so the next [`GameplaySystem::begin`] starts clean.
    fn cancel(&mut self);

    /// What this system draws while it has control, if anything.
    ///
    /// A full screen of its own, in the 1280×720 design box shared with the renderer's reference
    /// frame — not a caption inside the dialogue box. Elements are drawn in list order over
    /// whatever the story had on screen, so a system that wants a clean backdrop starts with a
    /// full-size [`Element::panel`](crate::view::Element::panel).
    ///
    /// Default implementation reports nothing: a system that needs no custom screen, or a
    /// headless run, draws nothing and is still legal. While such a system has control the
    /// player sees the story's last frame, which is the honest presentation — the engine will not
    /// invent a screen on the system's behalf.
    ///
    /// This is asked every frame, so it must be cheap: build a value, do not reshape text or
    /// touch the filesystem.
    fn view(&self) -> Option<crate::view::GameplayView> {
        None
    }

    /// Serialise the state that cannot be reconstructed from config alone.
    ///
    /// Returning `None` means "stateless": nothing is written to the save, and
    /// after a load the system is rebuilt by [`GameplaySystem::begin`]. That is
    /// correct for a system whose progress lives entirely in [`Stats`], such as a
    /// countdown derived from an `elapsed_ms` counter.
    ///
    /// Return `Some` for anything `begin` could not re-derive: a shuffled deck, a
    /// generated board, a sequence of inputs already accepted. Getting this wrong
    /// does not fail loudly — the save simply resumes with a fresh system — so
    /// the rule is: if `begin` cannot reproduce it, it belongs in here.
    ///
    /// The returned value is stored verbatim in the save file. Systems written by
    /// third parties choose their own format inside it; the core never looks.
    fn save(&self) -> Result<Option<ron::Value>, CoreError> {
        Ok(None)
    }

    /// Restore state previously produced by [`GameplaySystem::save`].
    ///
    /// Called immediately after [`GameplaySystem::begin`] during a load, so the
    /// config is already parsed and invariant state is established. Only the parts
    /// that change during play belong in here.
    ///
    /// The default implementation accepts `None` and **rejects** `Some`: if a save
    /// carries state for a system that never learned to restore it, the save and
    /// the code disagree, and resuming with silently discarded state is worse than
    /// refusing. Implement this method to accept state.
    fn restore(&mut self, state: Option<&ron::Value>) -> Result<(), CoreError> {
        match state {
            None => Ok(()),
            Some(_) => Err(CoreError::GameplayRestoreFailed {
                system: self.id().to_string(),
                message:
                    "this system does not implement restore, but the save carries state for it"
                        .to_owned(),
            }),
        }
    }
}

/// Encode a system's dynamic state into the opaque blob a save stores.
///
/// Exists because `ron` 0.11 has no `to_value`: turning a struct into a
/// [`ron::Value`] means going through its text form. Doing that here keeps every
/// gameplay implementation from repeating the same two-line dance, and keeps the
/// quirk in one place if a later `ron` grows the direct API.
///
/// Errors are plain strings so callers can wrap them in
/// [`CoreError::GameplaySaveFailed`] with their own system id.
pub fn encode_state<T: Serialize>(state: &T) -> Result<ron::Value, String> {
    let text = ron::to_string(state).map_err(|err| err.to_string())?;
    ron::from_str(&text).map_err(|err| err.to_string())
}

/// Decode state produced by [`encode_state`].
pub fn decode_state<T: serde::de::DeserializeOwned>(state: &ron::Value) -> Result<T, String> {
    state.clone().into_rust().map_err(|err| err.to_string())
}

/// The set of systems available to a running story.
///
/// Populated once at startup. Lookup by id keeps script references stable while
/// systems are replaced at compile time, not runtime.
#[derive(Default)]
pub struct GameplayRegistry {
    systems: BTreeMap<SystemId, Box<dyn GameplaySystem>>,
}

impl GameplayRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a system. Returns the previous system under the same id if one
    /// was already present, so double registration is visible rather than silent.
    pub fn register(&mut self, system: Box<dyn GameplaySystem>) -> Option<Box<dyn GameplaySystem>> {
        self.systems.insert(system.id(), system)
    }

    #[must_use]
    pub fn contains(&self, id: &SystemId) -> bool {
        self.systems.contains_key(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &SystemId> {
        self.systems.keys()
    }

    pub(crate) fn get_mut(&mut self, id: &SystemId) -> Option<&mut Box<dyn GameplaySystem>> {
        self.systems.get_mut(id)
    }

    /// Read-only lookup, used when saving to ask a suspended system for its state.
    #[must_use]
    pub fn get(&self, id: &SystemId) -> Option<&dyn GameplaySystem> {
        self.systems.get(id).map(AsRef::as_ref)
    }

    /// Read-only view lookup, for presentation.
    #[must_use]
    pub fn view_of(&self, id: &SystemId) -> Option<crate::view::GameplayView> {
        self.systems.get(id).and_then(|system| system.view())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_default_to_zero_and_saturate() {
        let mut stats = Stats::default();
        assert_eq!(stats.get("affection"), 0);

        stats.set("affection", 3);
        stats.add("affection", 2);
        assert_eq!(stats.get("affection"), 5);

        stats.set("guarded", i32::MAX);
        stats.add("guarded", 1);
        assert_eq!(stats.get("guarded"), i32::MAX, "must saturate, not wrap");
    }

    #[test]
    fn stats_iterate_in_stable_key_order() {
        let mut stats = Stats::default();
        stats.set("b", 2);
        stats.set("a", 1);
        let keys: Vec<&str> = stats.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["a", "b"]);
    }
}
