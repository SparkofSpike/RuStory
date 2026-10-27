//! Identifier types.
//!
//! Every id is a distinct newtype rather than a bare `String`. Mixing a node id
//! with an actor id is a mistake the compiler should catch, and a `String` alias
//! would let it through.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

define_id! {
    /// A scene node in the story graph. Nodes are the unit of authoring flow:
    /// a node plays its beats, then takes its exit.
    NodeId
}

define_id! {
    /// A character that can appear on stage.
    ActorId
}

define_id! {
    /// A registered gameplay system. The core treats this as an opaque handle: it
    /// looks systems up by id and never branches on which one it got.
    SystemId
}

define_id! {
    /// The key a gameplay system reports back to the narrative.
    ///
    /// The core never interprets it beyond looking it up in the node's outcome
    /// table, which is why adding a gameplay system needs no core change.
    OutcomeKey
}
