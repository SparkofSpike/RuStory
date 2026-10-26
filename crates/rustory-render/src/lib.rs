//! RuStory's renderer: a small 2D quad + text pipeline written directly against
//! `wgpu`. There is no third-party engine underneath — only the platform layer
//! (window, GPU backend, font rasterizer) is borrowed.
//!
//! Nothing in this crate is allowed to leak into [`rustory_core`]: the core owns
//! game state and knows nothing about pixels.

#![forbid(unsafe_code)]

/// Probes the GPU backend without opening a window.
///
/// This exists so `cargo run` can tell "no usable adapter on this machine" apart
/// from "our renderer has a bug" in one line, and so CI can smoke-test the
/// graphics stack headlessly.
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
    Ok(format!("{} [{} / {:?}]", info.name, info.driver, info.backend))
}

/// Probes the text stack: font discovery plus shaping readiness.
pub fn probe_text() -> String {
    let font_system = glyphon::FontSystem::new();
    format!("{} system fonts discovered", font_system.db().len())
}
