//! RuStory's renderer: a 2D quad and text pipeline written directly against
//! `wgpu`. There is no third-party engine underneath — only the platform layer
//! (window, GPU backend, font rasterizer) is borrowed.
//!
//! The crate consumes a [`Frame`], a plain data draw list. It has no notion of
//! stories, characters or choices: the application translates game state into
//! quads and text runs, and this crate turns them into pixels. That keeps game
//! semantics out of the renderer and lets layout be asserted in tests without a
//! GPU.
//!
//! Rendering can be verified headlessly: [`Renderer::render_offscreen`] returns
//! an [`Rgba8Image`] that can be saved as a PNG, which is how a frame gets
//! inspected in a place where nobody can look at a window.

#![forbid(unsafe_code)]

mod frame;
mod image;
mod renderer;

pub use frame::{FontChoice, Frame, Quad, Rect, Rgba, TextItem, TextureId, Uv};
pub use image::{ImageError, Rgba8Image};
pub use renderer::{RenderError, Renderer, TARGET_FORMAT};

/// Probes the GPU backend without opening a window.
///
/// Kept separate from [`Renderer`] so a startup check can report "no adapter on
/// this machine" in one line, distinct from a renderer bug.
pub fn probe_gpu() -> Result<String, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .map_err(|e| format!("no usable GPU adapter: {e}"))?;

    let info = adapter.get_info();
    Ok(format!(
        "{} [{} / {:?}]",
        info.name, info.driver, info.backend
    ))
}

/// Probes the text stack: font discovery.
pub fn probe_text() -> String {
    let font_system = glyphon::FontSystem::new();
    format!("{} system fonts discovered", font_system.db().len())
}
