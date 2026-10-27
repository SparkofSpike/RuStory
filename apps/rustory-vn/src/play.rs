//! The windowed player.
//!
//! This is the only place that knows a window exists. It owns the event loop,
//! translates platform input into narrative intents, and hands the resulting
//! draw list to the renderer. Game state stays in `rustory-vn-core`, which has never
//! heard of a mouse.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use rustory_vn_core::{GameplayInput, GameplayRegistry, Mode, Narrative};
use rustory_vn_render::{Renderer, TARGET_FORMAT};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::driver;
use crate::plugins;
use crate::scene::{HEIGHT, TextureBank, WIDTH};

/// Characters per second for the typewriter. Fast enough to stay out of the way,
/// slow enough that the reveal is visible.
const CHARS_PER_SEC: u32 = 45;

const HINT: &str = "Click / Space: advance   1-9: choose   F5: save   F9: load   Esc: quit";

pub fn run(args: &[String]) -> Result<(), String> {
    let mut story_path = None;
    let mut assets = PathBuf::from("assets");
    let mut save_path = PathBuf::from("save.ron");
    let mut font = None;
    let mut mute = false;
    let mut language: Option<String> = None;

    let mut index = 0;
    let mut plugin_flags: Vec<plugins::PluginRequest> = Vec::new();
    while index < args.len() {
        match args[index].as_str() {
            "--assets" => {
                index += 1;
                assets = PathBuf::from(args.get(index).ok_or("--assets needs a path")?);
            }
            "--save-slot" => {
                index += 1;
                save_path = PathBuf::from(args.get(index).ok_or("--save-slot needs a path")?);
            }
            "--font" => {
                index += 1;
                font = Some(args.get(index).ok_or("--font needs a family name")?.clone());
            }
            "--plugin" => {
                index += 1;
                plugin_flags.push(plugins::PluginRequest::parse(
                    args.get(index).ok_or("--plugin needs `name=path`")?,
                )?);
            }
            "--mute" => mute = true,
            "--language" => {
                index += 1;
                language = Some(
                    args.get(index)
                        .ok_or("--language needs a tag such as `en` or `zh-Hans`")?
                        .clone(),
                );
            }
            other if other.starts_with("--") => return Err(format!("unknown option `{other}`")),
            other => {
                if story_path.is_some() {
                    return Err(format!("unexpected argument `{other}`"));
                }
                story_path = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }

    let story_path = story_path.ok_or("play needs a story path")?;
    let story = rustory_vn_script::load_story(&story_path).map_err(|err| err.to_string())?;
    // A named language wins; otherwise the story picks, and `Narrative::new` resolves it.
    let story = match &language {
        Some(tag) => story.localised(tag).map_err(|err| match err {
            rustory_vn_core::CoreError::UnknownLanguage(tag) => {
                format!(
                    "no language `{tag}`; this story has {:?}",
                    story.languages()
                )
            }
            other => other.to_string(),
        })?,
        None => story,
    };

    // Modules before the narrative: a system that cannot load is a startup failure, not a
    // surprise three scenes in.
    let mut registry = driver::default_registry();
    let wanted: BTreeMap<rustory_vn_core::SystemId, PathBuf> = plugin_flags
        .into_iter()
        .map(|request| (request.name, request.path))
        .collect();
    let story_dir = story_path.parent().unwrap_or(std::path::Path::new("."));
    let report = plugins::load_into(&mut registry, &story, story_dir, &wanted);
    for line in report.summary() {
        println!("{line}");
    }
    if let Some((name, reason)) = report.failed.first() {
        return Err(format!("plugin `{name}` could not be loaded: {reason}"));
    }

    let narrative = Narrative::new(story).map_err(|err| err.to_string())?;

    let event_loop = EventLoop::new().map_err(|err| err.to_string())?;
    // Continuous redraw: the typewriter needs frames even when nothing moves.
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut player = Player::new(narrative, registry, assets, save_path, font, mute);
    event_loop
        .run_app(&mut player)
        .map_err(|err| err.to_string())
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    bank: TextureBank,
}

struct Player {
    narrative: Narrative,
    registry: GameplayRegistry,
    assets: PathBuf,
    /// Family name requested on the command line, applied once the renderer's font
    /// database exists. `None` means the platform's default sans-serif.
    font: Option<String>,
    /// Where F5 writes and F9 reads. A single slot keeps the mental model small;
    /// multiple slots would need a save browser before they were worth having.
    save_path: PathBuf,
    /// Transient message shown in the hint line, so a save or a failed load is
    /// visible without a console.
    notice: Option<String>,
    /// When the notice stops being shown.
    notice_until: Option<Instant>,

    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,

    /// Sound output. Silent when there is no device, which is a degraded run rather than
    /// a failed one.
    audio: crate::audio::Audio,

    last_tick: Instant,
    /// Inputs collected since the previous frame, handed to a gameplay system.
    pending_inputs: Vec<GameplayInput>,
}

impl Player {
    fn new(
        narrative: Narrative,
        registry: GameplayRegistry,
        assets: PathBuf,
        save_path: PathBuf,
        font: Option<String>,
        mute: bool,
    ) -> Self {
        // Audio lives beside the art, under its own folder: `<assets>/audio/`.
        let audio_dir = assets.join("audio");
        let audio = if mute {
            crate::audio::Audio::silent(&audio_dir)
        } else {
            crate::audio::Audio::new(&audio_dir)
        };
        // Said once at startup rather than silently: a developer wondering why the game is
        // quiet has the answer in the terminal instead of in a bug report.
        match &audio.unavailable {
            Some(reason) => eprintln!("rustory: {reason}"),
            None => println!(
                "audio: {} file(s) in {}",
                audio.library_size(),
                audio_dir.display()
            ),
        }
        Self {
            narrative,
            registry,
            assets,
            font,
            save_path,
            notice: None,
            notice_until: None,
            window: None,
            gpu: None,
            audio,
            last_tick: Instant::now(),
            pending_inputs: Vec::new(),
        }
    }

    /// Write the current state to the save slot.
    fn save_to_slot(&mut self) {
        let result = self.narrative.save(&self.registry).and_then(|save| {
            save.to_ron()
                .map_err(|err| rustory_vn_core::CoreError::GameplaySaveFailed {
                    system: "save".into(),
                    message: err.to_string(),
                })
        });

        match result {
            Ok(text) => match std::fs::write(&self.save_path, text) {
                Ok(()) => self.set_notice(format!("Saved to {}", self.save_path.display())),
                Err(err) => self.set_notice(format!("Save failed: {err}")),
            },
            Err(err) => self.set_notice(format!("Save failed: {err}")),
        }
    }

    /// Show a message on the hint line for a few seconds.
    fn set_notice(&mut self, message: String) {
        self.notice = Some(message);
        self.notice_until = Some(Instant::now() + std::time::Duration::from_secs(3));
    }

    /// Replace the running session with whatever is in the save slot.
    ///
    /// A failed load leaves the current session untouched: refusing to load is
    /// recoverable, while half-restoring would destroy the run in progress.
    fn load_from_slot(&mut self) {
        let outcome = (|| -> Result<(), String> {
            let text = std::fs::read_to_string(&self.save_path)
                .map_err(|err| format!("cannot read {}: {err}", self.save_path.display()))?;
            let save =
                rustory_vn_core::SaveState::from_ron(&text).map_err(|err| err.to_string())?;
            let story = self.narrative.story().clone();
            let restored = Narrative::from_save(story, &save, &mut self.registry)
                .map_err(|err| err.to_string())?;
            self.narrative = restored;
            // A line revealed before the load must not continue revealing from a
            // stale accumulator.
            self.last_tick = Instant::now();
            self.pending_inputs.clear();
            Ok(())
        })();

        if outcome.is_ok() {
            // The restored scene has a track, but nothing "changed": the events for this
            // position were consumed long ago. Ask the narrative what should be playing
            // rather than waiting for the next `Music` beat.
            let track = self.narrative.music().map(str::to_owned);
            self.audio.sync_to(track.as_deref());
        }

        self.set_notice(match outcome {
            Ok(()) => format!("Loaded {}", self.save_path.display()),
            Err(err) => format!("Load failed: {err}"),
        });
    }

    fn init_gpu(&mut self, window: Arc<Window>) -> Result<(), String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|err| format!("cannot create a surface: {err}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|err| format!("no adapter can present to this window: {err}"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rustory window device"),
            ..Default::default()
        }))
        .map_err(|err| format!("cannot create a device: {err}"))?;

        let caps = surface.get_capabilities(&adapter);
        if !caps.formats.contains(&TARGET_FORMAT) {
            // The renderer builds one pipeline for one format. Saying so beats
            // rendering a blank window on an exotic surface.
            return Err(format!(
                "surface does not support {TARGET_FORMAT:?}; supported: {:?}",
                caps.formats
            ));
        }

        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: TARGET_FORMAT,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            // Auto resolves to plain sRGB for an 8-bit format, which is what a
            // visual novel wants: no HDR, no wide gamut, no surprises.
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let mut renderer = Renderer::from_device(device, queue, config.width, config.height)
            .map_err(|err| err.to_string())?;
        let bank = driver::load_textures(&mut renderer, &self.assets);

        // Project fonts are loaded before the family is chosen, so `--font` can
        // name a file the project ships rather than only an installed one.
        let loaded = driver::load_fonts(&mut renderer, &self.assets);
        match driver::choose_font(&mut renderer, self.font.as_deref(), &loaded) {
            Ok(Some(family)) => {
                if self.font.is_none() {
                    // Say it once: a silently different typeface is confusing when
                    // someone later wonders why the file they added took effect.
                    eprintln!("rustory: using project font `{family}`");
                }
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("rustory: {err}");
                self.set_notice(format!("Font problem: {err}"));
            }
        }

        self.gpu = Some(Gpu {
            surface,
            config,
            renderer,
            bank,
        });
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        if width == 0 || height == 0 {
            return;
        }
        gpu.config.width = width;
        gpu.config.height = height;
        gpu.surface.configure(gpu.renderer.device(), &gpu.config);
        gpu.renderer.resize(width, height);
    }

    /// Advance game state by `dt_ms` and draw.
    fn update_and_draw(&mut self) {
        let now = Instant::now();
        let dt_ms = (now - self.last_tick).as_millis().min(100) as u32;
        self.last_tick = now;

        if self.notice_until.is_some_and(|deadline| now >= deadline) {
            self.notice = None;
            self.notice_until = None;
        }

        // A gameplay system drives its own clock, so the narrative is ticked
        // separately: while suspended there is no line to reveal anyway.
        self.narrative.tick(dt_ms, CHARS_PER_SEC);

        if matches!(self.narrative.mode(), Mode::Suspended { .. }) {
            let inputs = std::mem::take(&mut self.pending_inputs);
            if let Err(err) = self
                .narrative
                .step_gameplay(&mut self.registry, dt_ms, &inputs)
            {
                eprintln!("rustory: gameplay error: {err}");
            }
        } else {
            // Inputs that arrived while nothing was listening must not leak into
            // the next scene.
            self.pending_inputs.clear();
        }

        // Drain before ticking: the events describe what this frame decided, and the
        // director needs them before it advances the fades those events started.
        //
        // This is the first real consumer of `NarrativeEvent`. The queue existed and
        // nothing read it, which is the difference between a design that supports a
        // feature and one that has it.
        let events = self.narrative.take_events();
        self.audio.handle(&events);
        self.audio.tick(dt_ms);

        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        let view = self.narrative.gameplay_view(&self.registry);
        // The window may be a different size than the logical resolution; the
        // scene is laid out at the logical size and scaled by the surface.
        let mut frame = driver::build_frame(
            &self.narrative,
            &gpu.bank,
            view.as_ref(),
            // A save notice takes the hint line for a few seconds, then the usual
            // key reminder comes back. Overlaying both would make neither readable.
            Some(self.notice.as_deref().unwrap_or(HINT)),
        );
        frame.width = gpu.config.width;
        frame.height = gpu.config.height;
        letterbox(&mut frame);

        // A lost or outdated surface is recoverable: reconfigure and try again
        // next frame instead of tearing the process down.
        let acquired = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            _ => {
                gpu.surface.configure(gpu.renderer.device(), &gpu.config);
                return;
            }
        };
        let view = acquired
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        if let Err(err) = gpu.renderer.render_to(&frame, &view) {
            eprintln!("rustory: render error: {err}");
            return;
        }
        // Presentation moved from SurfaceTexture to Queue in wgpu 30.
        gpu.renderer.queue().present(acquired);
    }

    fn on_confirm(&mut self) {
        if matches!(self.narrative.mode(), Mode::Suspended { .. }) {
            self.pending_inputs.push(GameplayInput::Confirm);
            return;
        }
        if let Err(err) = driver::click(&mut self.narrative, &mut self.registry) {
            eprintln!("rustory: {err}");
        }
    }

    fn on_select(&mut self, index: usize) {
        match self.narrative.mode() {
            Mode::Suspended { .. } => self.pending_inputs.push(GameplayInput::Select { index }),
            Mode::AwaitingChoice { choices, .. } => {
                if index < choices.len()
                    && let Err(err) = self.narrative.choose(index)
                {
                    eprintln!("rustory: {err}");
                }
            }
            _ => {}
        }
    }
}

/// Fit a logical-resolution scene into whatever the window actually is.
///
/// The scene is authored at [`WIDTH`]x[`HEIGHT`]; a different window size should
/// not rescale the dialogue into illegibility, so the frame is centred and
/// painted over black bars instead.
fn letterbox(frame: &mut rustory_vn_render::Frame) {
    if frame.width == WIDTH && frame.height == HEIGHT {
        return;
    }
    let scale = (frame.width as f32 / WIDTH as f32).min(frame.height as f32 / HEIGHT as f32);
    let offset_x = (frame.width as f32 - WIDTH as f32 * scale) / 2.0;
    let offset_y = (frame.height as f32 - HEIGHT as f32 * scale) / 2.0;

    for quad in &mut frame.quads {
        quad.rect.x = quad.rect.x * scale + offset_x;
        quad.rect.y = quad.rect.y * scale + offset_y;
        quad.rect.width *= scale;
        quad.rect.height *= scale;
    }
    for text in &mut frame.texts {
        text.x = text.x * scale + offset_x;
        text.y = text.y * scale + offset_y;
        text.font_size *= scale;
        text.line_height *= scale;
        if text.max_width.is_finite() {
            text.max_width *= scale;
        }
    }
}

impl ApplicationHandler for Player {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("RuStory")
            .with_inner_size(winit::dpi::LogicalSize::new(WIDTH, HEIGHT));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                eprintln!("rustory: cannot create a window: {err}");
                event_loop.exit();
                return;
            }
        };

        if let Err(err) = self.init_gpu(window.clone()) {
            eprintln!("rustory: {err}");
            event_loop.exit();
            return;
        }

        self.window = Some(window);
        self.last_tick = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size.width, size.height),
            WindowEvent::RedrawRequested => self.update_and_draw(),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.on_confirm(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                    PhysicalKey::Code(KeyCode::F5) => self.save_to_slot(),
                    PhysicalKey::Code(KeyCode::F9) => self.load_from_slot(),
                    PhysicalKey::Code(KeyCode::Space | KeyCode::Enter) => self.on_confirm(),
                    PhysicalKey::Code(code) => {
                        if let Some(index) = digit_index(code) {
                            self.on_select(index);
                        }
                    }
                    PhysicalKey::Unidentified(_) => {}
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Map the top-row digits to zero-based option indices.
fn digit_index(code: KeyCode) -> Option<usize> {
    match code {
        KeyCode::Digit1 => Some(0),
        KeyCode::Digit2 => Some(1),
        KeyCode::Digit3 => Some(2),
        KeyCode::Digit4 => Some(3),
        KeyCode::Digit5 => Some(4),
        KeyCode::Digit6 => Some(5),
        KeyCode::Digit7 => Some(6),
        KeyCode::Digit8 => Some(7),
        KeyCode::Digit9 => Some(8),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_map_to_zero_based_indices() {
        assert_eq!(digit_index(KeyCode::Digit1), Some(0));
        assert_eq!(digit_index(KeyCode::Digit9), Some(8));
        assert_eq!(digit_index(KeyCode::KeyA), None);
    }

    #[test]
    fn letterboxing_keeps_a_matching_size_untouched() {
        let mut frame = rustory_vn_render::Frame::new(WIDTH, HEIGHT);
        frame.push_quad(rustory_vn_render::Quad::solid(
            rustory_vn_render::Rect::new(10.0, 20.0, 30.0, 40.0),
            rustory_vn_render::Rgba::rgb(255, 255, 255),
        ));
        let before = frame.quads[0].rect;
        letterbox(&mut frame);
        assert_eq!(frame.quads[0].rect, before);
    }

    #[test]
    fn letterboxing_scales_and_centres_a_larger_window() {
        let mut frame = rustory_vn_render::Frame::new(2560, 1440);
        frame.push_quad(rustory_vn_render::Quad::solid(
            rustory_vn_render::Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
            rustory_vn_render::Rgba::rgb(255, 255, 255),
        ));
        letterbox(&mut frame);

        let rect = frame.quads[0].rect;
        assert!((rect.width - 2560.0).abs() < 0.01, "full width at 2x");
        assert!((rect.height - 1440.0).abs() < 0.01, "full height at 2x");
        assert!((rect.x).abs() < 0.01);
    }
}
