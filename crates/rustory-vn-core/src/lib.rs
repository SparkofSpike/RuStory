//! RuStory narrative core.
//!
//! This crate owns the story model, the narrative state machine and the gameplay
//! contract. It has no rendering dependency and no window dependency, so every
//! behaviour here is testable in memory — which is also why it is the only crate
//! with meaningful unit tests.
//!
//! # The shape of a story
//!
//! ```text
//! Story ── start ──▶ Node ── beats ──▶ [Say | ShowActor | HideActor | SetBackground]
//!                     │
//!                     └── exit ─────▶ [End | Goto | Choose | Gameplay]
//! ```
//!
//! A [`Narrative`] walks that graph. It pauses when a line needs revealing, when
//! the player must choose, and when a gameplay system takes over.
//!
//! # Where gameplay plugs in
//!
//! A node can hand control to any registered [`GameplaySystem`]. The core hands
//! over an opaque config and receives back an [`OutcomeKey`]; the *script* maps
//! that key to the next node. Adding a gameplay system therefore requires no
//! change to this crate.
//!
//! # Example
//!
//! ```
//! use rustory_vn_core::prelude::*;
//! use std::collections::BTreeMap;
//!
//! let mut nodes = BTreeMap::new();
//! nodes.insert(NodeId::from("start"), Node {
//!     beats: vec![
//!         Beat::SetBackground { image: Some("school.ron".into()) },
//!         Beat::Say { speaker: None, text: "The bell rings.".into() },
//!     ],
//!     exit: Exit::End { ending: None },
//! });
//!
//! let mut narrative = Narrative::new(Story::new("start", nodes))?;
//!
//! // The background applied immediately; the line is still revealing.
//! assert_eq!(narrative.stage().background(), Some("school.ron"));
//! assert!(matches!(narrative.mode(), Mode::Revealing));
//!
//! narrative.tick(1000, 0); // 0 = instant
//! assert!(matches!(narrative.mode(), Mode::AwaitingAdvance));
//! narrative.advance()?;
//! assert!(matches!(narrative.mode(), Mode::Finished { .. }));
//! # Ok::<(), rustory_vn_core::CoreError>(())
//! ```

#![forbid(unsafe_code)]

mod error;
mod event;
mod gameplay;
mod ids;
mod narrative;
mod save;
mod slots;
mod stage;
mod story;
mod text;
mod view;

pub use error::CoreError;
pub use event::{NarrativeEvent, TickReport};
pub use gameplay::{
    ChoiceMade, GameplayContext, GameplayInput, GameplayRegistry, GameplaySystem,
    NarrativeSnapshot, PointerPhase, Stats, decode_state, encode_state,
};
pub use ids::{ActorId, NodeId, OutcomeKey, StoryId, SystemId};
pub use narrative::{LineView, Mode, Narrative};
pub use save::{GameplaySave, SAVE_VERSION, SaveState};
pub use slots::{
    DEFAULT_SLOTS, SaveSlot, SlotIndex, SlotLayout, SlotSummary, Thumbnail, damaged, summarise,
};
pub use stage::{ActorOnStage, Stage};
pub use story::{Beat, Choice, Exit, GameplaySpec, MissingText, Node, Slot, Story, StoryMeta};
pub use text::{LocaleTable, Locales, Text, is_valid_language_tag};
pub use view::{Align, Element, GameplayView, Tone, VIEW_HEIGHT, VIEW_WIDTH, ViewRect};

/// The types almost every caller needs.
pub mod prelude {
    pub use crate::{
        ActorId, Align, Beat, Choice, ChoiceMade, CoreError, DEFAULT_SLOTS, Element, Exit,
        GameplayContext, GameplayInput, GameplayRegistry, GameplaySave, GameplaySpec,
        GameplaySystem, GameplayView, LineView, LocaleTable, Locales, MissingText, Mode, Narrative,
        NarrativeEvent, NarrativeSnapshot, Node, NodeId, OutcomeKey, PointerPhase, SAVE_VERSION,
        SaveSlot, SaveState, Slot, SlotIndex, SlotLayout, SlotSummary, Stage, Stats, Story,
        StoryId, StoryMeta, SystemId, Text, Thumbnail, TickReport, Tone, VIEW_HEIGHT, VIEW_WIDTH,
        ViewRect, damaged, decode_state, encode_state, summarise,
    };
}
