//! The stage: the small, explicit world the narrative drives.
//!
//! There is deliberately no general-purpose entity/component store here. A visual
//! novel needs a background and a handful of actors; modelling that as a dynamic
//! component map would buy flexibility nobody asked for and cost every reader the
//! ability to see the state shape. The fields below *are* the state shape, and
//! the compiler checks every use.
//!
//! Gameplay systems do not read this type. What they share with the narrative is
//! [`crate::gameplay::Stats`] — a narrow, declared surface — so a gameplay system
//! cannot quietly start depending on sprite positions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::ActorId;
use crate::story::{Beat, Slot};

/// An actor currently on stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorOnStage {
    pub pose: String,
    pub slot: Slot,
}

/// Everything visible right now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    background: Option<String>,
    actors: BTreeMap<ActorId, ActorOnStage>,
}

impl Stage {
    #[must_use]
    pub fn background(&self) -> Option<&str> {
        self.background.as_deref()
    }

    #[must_use]
    pub fn actors(&self) -> &BTreeMap<ActorId, ActorOnStage> {
        &self.actors
    }

    #[must_use]
    pub fn actor(&self, id: &ActorId) -> Option<&ActorOnStage> {
        self.actors.get(id)
    }

    /// Apply one beat's effect on the stage.
    ///
    /// Returns `true` if the stage changed. `Say` never changes the stage; it is
    /// handled by the narrative state machine, which is the reason this function
    /// takes a `&Beat` and matches exhaustively instead of taking an effect enum.
    pub fn apply(&mut self, beat: &Beat) -> bool {
        match beat {
            Beat::Say { .. } => false,
            Beat::ShowActor { actor, pose, slot } => {
                self.actors.insert(
                    actor.clone(),
                    ActorOnStage {
                        pose: pose.clone(),
                        slot: *slot,
                    },
                );
                true
            }
            Beat::HideActor { actor } => self.actors.remove(actor).is_some(),
            Beat::SetBackground { image } => {
                self.background.clone_from(image);
                true
            }
        }
    }

    pub fn clear(&mut self) {
        self.background = None;
        self.actors.clear();
    }
}
