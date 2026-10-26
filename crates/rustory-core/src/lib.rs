//! RuStory narrative core.
//!
//! This crate owns the story model and the narrative state machine. It is
//! intentionally free of Bevy and of any rendering dependency so that it can be
//! unit-tested without a GPU or a window.

#![forbid(unsafe_code)]

/// Placeholder kept only until the first real module lands.
pub fn crate_name() -> &'static str {
    "rustory-core"
}
