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
//! Only [`Stats`] and its own fields. It cannot see the stage, the story graph or
//! the renderer, because none of those are in [`GameplayContext`]. If a system
//! needs new shared state, that is a deliberate widening of this contract and
//! belongs in an ADR — not in a `&mut World` escape hatch.

use std::collections::BTreeMap;

use crate::error::CoreError;
use crate::ids::{OutcomeKey, SystemId};

/// Named integer values shared between the narrative and gameplay systems.
///
/// This is intentionally the narrowest useful surface: counters, scores, flags.
/// Keyed by string because different stories invent different attributes, but the
/// value type is fixed — a bag of `Any` here would be the exact dynamic-shape
/// improvisation this project exists to avoid.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

    #[must_use]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameplayInput {
    /// The player clicked, tapped or pressed confirm.
    Confirm,
    /// The player chose option `index` from whatever the system presented.
    Select { index: usize },
}

/// What a gameplay system is allowed to see and change.
pub struct GameplayContext<'a> {
    /// Milliseconds since the previous step. Wall-clock stays in the app layer.
    pub dt_ms: u32,
    /// Inputs collected since the previous step.
    pub inputs: &'a [GameplayInput],
    /// Shared counters. The only narrative-adjacent state gameplay may touch.
    pub stats: &'a mut Stats,
}

/// A pluggable gameplay system.
///
/// Implementors live in their own crate. Adding one means registering it; the
/// narrative core compiles unchanged.
pub trait GameplaySystem {
    /// The id scripts use to invoke this system.
    fn id(&self) -> SystemId;

    /// Called once when the script hands control over, with that node's config.
    ///
    /// Parsing happens here rather than in the loader so the core stays ignorant
    /// of the concrete config type. Returning an error aborts the handover and
    /// surfaces to the caller; it must not leave half-initialized state behind.
    fn begin(&mut self, config: Option<&ron::Value>) -> Result<(), CoreError>;

    /// Advance by `dt_ms`. Return `Some(outcome)` when the system is finished,
    /// `None` to keep control.
    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey>;

    /// The narrative is aborting this system (story closed, save loaded past it).
    /// Must reset so the next [`GameplaySystem::begin`] starts clean.
    fn cancel(&mut self);

    /// A short label for the system's current on-screen prompt, if it has one.
    ///
    /// Presentation-neutral: the renderer decides how to draw it. Default
    /// implementation reports nothing, so simple systems do not implement it.
    fn prompt(&self) -> Option<String> {
        None
    }
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
    pub fn register(
        &mut self,
        system: Box<dyn GameplaySystem>,
    ) -> Option<Box<dyn GameplaySystem>> {
        self.systems.insert(system.id(), system)
    }

    #[must_use]
    pub fn contains(&self, id: &SystemId) -> bool {
        self.systems.contains_key(id)
    }

    #[must_use]
    pub fn ids(&self) -> impl Iterator<Item = &SystemId> {
        self.systems.keys()
    }

    pub(crate) fn get_mut(&mut self, id: &SystemId) -> Option<&mut Box<dyn GameplaySystem>> {
        self.systems.get_mut(id)
    }

    /// Read-only prompt lookup, for presentation.
    #[must_use]
    pub fn prompt_of(&self, id: &SystemId) -> Option<String> {
        self.systems.get(id).and_then(|system| system.prompt())
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
