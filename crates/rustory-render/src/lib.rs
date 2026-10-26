//! RuStory's renderer: a small 2D quad + text pipeline written directly against
//! `wgpu`. There is no third-party engine underneath — only the platform layer
//! (window, GPU backend, font rasterizer) is borrowed.
//!
//! Nothing in this crate is allowed to leak into [`rustory_core`]: the core owns
//! game state and knows nothing about pixels.

#![forbid(unsafe_code)]

/// Placeholder kept only until the first real module lands.
pub fn crate_name() -> &'static str {
    "rustory-render"
}
