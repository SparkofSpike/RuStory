//! Live preview: the real engine rendering the real story into a texture.
//!
//! # Why not draw the layout in egui
//!
//! A preview is only worth having if the thing it shows is what will ship. Drawing an
//! approximation in egui would drift from `rustory-vn-render` the first time either side
//! changed, and would be trusted precisely because it looked plausible. So the preview
//! runs `rustory_vn_render::Renderer` — the same code `rustory play` uses — into an
//! offscreen texture, and egui displays that texture as an image.
//!
//! # What is simulated, and what is not
//!
//! Rendered: the frame the story would produce at a chosen point — background,
//! sprites, dialogue box, choices, gameplay prompt.
//!
//! Simulated: *which* point. The editor advances a real [`Narrative`] with the same
//! `click` semantics the CLI uses, so "step 3" in the editor and "step 3" on the
//! command line mean the same state. Nothing about the frame itself is faked.

use rustory_vn_core::{GameplayInput, GameplayRegistry, Mode, Narrative, NodeId, Story};
use rustory_vn_edit::prelude::*;
use rustory_vn_render::{Frame, Rect, Renderer, Rgba8Image};

/// Logical resolution the scene is authored at.
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;

/// Asset directory, relative to the story file. See [`AssetPaths`].
/// Everything the preview needs to produce one image.
///
/// # Why the image goes through the CPU
///
/// The first implementation rendered into a GPU texture and registered it with egui as
/// a native texture. That is the efficient path and it did not work: the texture the
/// engine drew into and the texture egui sampled were not the same object, so the panel
/// showed a stale blank frame. Debugging it cost more than the readback does.
///
/// A preview does not need sixty frames per second — it needs to be correct when the
/// document changes. So the frame is rendered offscreen, read back, and handed to egui
/// as an ordinary texture. A few milliseconds on edit, nothing while idle, and the
/// display path is the one egui is actually designed for.
/// One thing a player did, kept so a position can be replayed instead of mutated.
///
/// The preview is rebuilt from a path whenever the document changes: an edit, an undo or
/// a switch of node all invalidate an in-memory position, and replaying the actions is
/// the one way to be sure the frame matches the text on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Advance: finish a line, move on, or resolve a gameplay handover the way the CLI
    /// would.
    Click,
    /// Take a branch. A path that no longer fits falls back to [`Step::Click`], because
    /// editing the story can move a choice out from under it.
    Choose(usize),
}

/// What the frame means for the person looking at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewReport {
    /// Where the preview is, as one line of text.
    pub status: String,
    /// Labels of the choices on offer; empty when nothing is being asked.
    pub choices: Vec<String>,
    /// Whether the story has ended.
    pub finished: bool,
}

pub struct Preview {
    renderer: Option<Renderer>,

    /// Uploaded art, cached.
    ///
    /// Re-uploading every frame was the original implementation and it was a memory
    /// bomb: three 1280x720 textures per frame is roughly 11MB, sixty times a second.
    /// The failure mode was not a crash but *black textures*, which looked like a
    /// renderer bug for far longer than it should have.
    bank: Option<(std::path::PathBuf, TextureBank)>,

    /// The image egui draws. `None` until the first render.
    image: Option<egui::TextureHandle>,

    /// What the cached image was rendered from, so an unchanged document does not
    /// re-render. `(story checksum, start node, the steps taken)`.
    last_key: Option<(u64, NodeId, Vec<Step>)>,

    /// The last report, so a cache hit answers with the same thing as the render that
    /// filled it. Recomputing it on a hit produced a shorter message, which made the
    /// header flicker between two formats.
    last_report: Option<PreviewReport>,

    /// Why rendering is unavailable, if it is. Shown to the user instead of an empty
    /// panel, because "no picture" and "no GPU" are different problems.
    pub unavailable: Option<String>,
}

impl Default for Preview {
    fn default() -> Self {
        Self::new()
    }
}

impl Preview {
    #[must_use]
    pub fn new() -> Self {
        Self {
            renderer: None,
            bank: None,
            image: None,
            last_key: None,
            last_report: None,
            unavailable: None,
        }
    }

    /// Set up the GPU side, once.
    ///
    /// Called from the first frame, when `eframe` has a device. Failure is recorded
    /// rather than fatal: an editor that can still edit text without a GPU is more
    /// useful than one that refuses to start.
    pub fn ensure_ready(&mut self, cc: &eframe::CreationContext<'_>) {
        if self.renderer.is_some() || self.unavailable.is_some() {
            return;
        }

        // Reuse eframe's device rather than creating a second GPU context. The editor
        // and the preview are one process; two devices would double the driver's work
        // for no benefit.
        let renderer = match cc.wgpu_render_state.as_ref() {
            Some(render_state) => Renderer::from_device(
                render_state.device.clone(),
                render_state.queue.clone(),
                WIDTH,
                HEIGHT,
            ),
            None => Renderer::new_headless(WIDTH, HEIGHT),
        };

        match renderer {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(err) => self.unavailable = Some(format!("preview renderer unavailable: {err}")),
        }
    }

    #[must_use]
    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.image.as_ref()
    }

    /// Forget the cached frame and art.
    ///
    /// Called when the document changes identity — opened, created, saved elsewhere.
    /// Without it, a new story would show the previous one's preview until something
    /// happened to change the cache key.
    pub fn reset(&mut self) {
        self.image = None;
        self.last_key = None;
        self.last_report = None;
        self.bank = None;
    }

    /// Render the story as it would appear after `steps` interactions.
    ///
    /// Returns a short status line for the panel header: which node the preview is
    /// showing, so the user is never looking at an image without knowing where in the
    /// story it came from.
    ///
    /// Skips the work when neither the story nor the step changed — the common case
    /// while the user is reading or typing in a field that has not been committed.
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        story: &Story,
        start: &NodeId,
        steps: &[Step],
        paths: &AssetPaths,
    ) -> Result<PreviewReport, String> {
        // The start node and the path both belong in the key: selecting a different node
        // or taking a different branch has to produce a different frame even when the
        // document is unchanged.
        let key = (story.checksum(), start.clone(), steps.to_vec());

        if self.last_key.as_ref() == Some(&key)
            && self.image.is_some()
            && let Some(report) = self.last_report.clone()
        {
            return Ok(report);
        }

        let Some(renderer) = self.renderer.as_mut() else {
            return Err("preview renderer is not ready".to_owned());
        };

        let mut narrative = Narrative::at(story.clone(), start).map_err(|err| err.to_string())?;
        let mut registry = default_registry();

        for action in steps {
            if matches!(narrative.mode(), Mode::Finished { .. }) {
                break;
            }
            match action {
                Step::Click => step(&mut narrative, &mut registry)?,
                Step::Choose(index) => {
                    if matches!(narrative.mode(), Mode::AwaitingChoice { .. }) {
                        narrative.choose(*index).map_err(|err| err.to_string())?;
                    } else {
                        // The path was recorded against a different version of the
                        // story. Advancing is the closest honest reading of it.
                        step(&mut narrative, &mut registry)?;
                    }
                }
            }
        }

        // Show the line in full: a preview of half a sentence tells the user nothing
        // about how the finished frame looks.
        narrative.tick(0, 0);
        if matches!(narrative.mode(), Mode::Suspended { .. }) {
            // Perform the gameplay handover so the system's prompt exists.
            let _ = narrative.step_gameplay(&mut registry, 0, &[]);
        }

        let (bank, image_count) = bank_for(&mut self.bank, renderer, paths);
        let prompt = narrative.gameplay_prompt(&registry);
        let frame = build_frame(&narrative, &bank, prompt.as_deref());

        let image = renderer
            .render_offscreen(&frame)
            .map_err(|err| err.to_string())?;

        let pixels = to_egui(&image);
        match self.image.as_mut() {
            Some(handle) => handle.set(pixels, egui::TextureOptions::LINEAR),
            None => {
                self.image =
                    Some(ctx.load_texture("rustory-preview", pixels, egui::TextureOptions::LINEAR));
            }
        }
        self.last_key = Some(key);

        // The image count is part of the header because a blank preview has two
        // very different causes — the layout produced nothing, or the art was not
        // found — and the count separates them instantly. When it is zero, the paths
        // that were searched are listed too; "0 images" alone leaves the user
        // guessing where the editor looked.
        let art = if image_count == 0 {
            format!(
                "0 images (looked in {})",
                paths
                    .candidates()
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            format!("{image_count} image(s)")
        };

        // Choices and the ending flag are read while the story is still in hand: the
        // panel needs them to draw buttons, and a cached string cannot say which branch
        // is on offer.
        let choices = match narrative.mode() {
            Mode::AwaitingChoice { choices, .. } => {
                choices.iter().map(|choice| choice.label.clone()).collect()
            }
            _ => Vec::new(),
        };
        let finished = matches!(narrative.mode(), Mode::Finished { .. });

        let report = PreviewReport {
            status: format!(
                "{} · {} primitives · {art}",
                status_of(&narrative),
                frame.primitive_count()
            ),
            choices,
            finished,
        };
        self.last_report = Some(report.clone());
        Ok(report)
    }
}

/// What the preview is showing, as a line of text.
///
/// Read from the narrative that produced the frame instead of rebuilding it from the
/// start: a second `Narrative` would be a second opinion about where the preview is, and
/// the header would then disagree with the picture directly below it.
fn status_of(narrative: &Narrative) -> String {
    match narrative.mode() {
        Mode::Finished { ending } => match ending {
            Some(ending) => format!("end ({ending})"),
            None => "end".to_owned(),
        },
        Mode::Suspended { spec } => format!("{} at `{}`", spec.system, narrative.cursor()),
        _ => format!(
            "`{}` beat {} of {}",
            narrative.cursor(),
            narrative.beat_index(),
            narrative.current_node().beats.len()
        ),
    }
}

/// Reuse the uploaded art when the resolved directory has not changed.
///
/// A free function rather than a method so it can be called while `self.renderer` is
/// borrowed mutably; the borrow checker is right that a method would alias.
fn bank_for(
    cache: &mut Option<(std::path::PathBuf, TextureBank)>,
    renderer: &mut Renderer,
    paths: &AssetPaths,
) -> (TextureBank, usize) {
    let resolved = paths.resolve();

    if let Some((dir, bank)) = cache.take() {
        if resolved.as_deref() == Some(dir.as_path()) {
            let count = bank.len();
            *cache = Some((dir, bank.clone()));
            return (bank, count);
        }
    }

    let (bank, count) = textures(renderer, paths);
    if let Some(dir) = resolved {
        *cache = Some((dir, bank.clone()));
    }
    (bank, count)
}

/// Convert a rendered image into an egui texture.
///
/// Lives here rather than in `rustory-vn-render` on purpose: the renderer must not know
/// that egui exists, or `rustory play` would carry a GUI dependency.
fn to_egui(image: &Rgba8Image) -> egui::ColorImage {
    let pixels = image
        .data()
        .chunks_exact(4)
        .map(|pixel| egui::Color32::from_rgba_unmultiplied(pixel[0], pixel[1], pixel[2], pixel[3]))
        .collect();
    egui::ColorImage {
        size: [image.width() as usize, image.height() as usize],
        pixels,
        source_size: egui::vec2(image.width() as f32, image.height() as f32),
    }
}

/// The engine's own gameplay systems, duplicated here because this binary is a
/// separate program.
///
/// Duplication is the cost of the editor being its own executable. It is a real cost:
/// registering a system here and forgetting the CLI would make the two disagree. The
/// mitigation is that `rustory ops`-style drift is impossible for *editing*, and a
/// missing system surfaces immediately as `UnknownGameplaySystem` when the preview
/// reaches that node.
pub fn default_registry() -> GameplayRegistry {
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(rustory_vn_gameplay_timer::TimedChoice::new()));
    registry.register(Box::new(rustory_vn_gameplay_quiz::Quiz::new()));
    registry
}

/// One interaction, whatever the current mode demands.
///
/// Mirrors `rustory`'s driver so a preview step and a CLI step land in the same place.
pub fn step(narrative: &mut Narrative, registry: &mut GameplayRegistry) -> Result<(), String> {
    match narrative.mode().clone() {
        Mode::Revealing => {
            narrative.tick(0, 0);
            narrative.advance().map_err(|err| err.to_string())
        }
        Mode::AwaitingAdvance => narrative.advance().map_err(|err| err.to_string()),
        Mode::AwaitingChoice { choices, .. } => {
            if choices.is_empty() {
                return Err("choice node offers no choices".into());
            }
            narrative.choose(0).map_err(|err| err.to_string())
        }
        Mode::Suspended { .. } => {
            let input = [GameplayInput::Select { index: 0 }];
            for _ in 0..10_000 {
                narrative
                    .step_gameplay(registry, 16, &input)
                    .map_err(|err| err.to_string())?;
                if !matches!(narrative.mode(), Mode::Suspended { .. }) {
                    return Ok(());
                }
            }
            Err("gameplay system did not finish".into())
        }
        Mode::Finished { .. } => Ok(()),
    }
}

/// Build the frame with the same layout the shipping game uses.
///
/// This intentionally duplicates `apps/rustory-vn/src/scene.rs` rather than sharing it.
/// The alternative — hoisting layout into `rustory-vn-render` — would put story concepts
/// into the renderer, which ADR 0001 forbids. The duplication is real and is recorded
/// as a known cost; the guarantee that keeps it honest is that both call the same
/// `rustory_vn_render` primitives, so a change to the *engine* shows up in both.
pub fn build_frame(narrative: &Narrative, bank: &TextureBank, prompt: Option<&str>) -> Frame {
    use rustory_vn_render::{Quad, TextItem};

    const DIALOGUE: Rect = Rect::new(60.0, 510.0, 1160.0, 180.0);
    const CHOICE_WIDTH: f32 = 620.0;
    const CHOICE_HEIGHT: f32 = 66.0;
    const CHOICE_TOP: f32 = 210.0;
    const CHOICE_GAP: f32 = 18.0;
    const SPRITE_SCALE: f32 = 0.75;

    const COL_BOX: rustory_vn_render::Rgba = rustory_vn_render::Rgba::rgba(8, 10, 16, 236);
    const COL_TEXT: rustory_vn_render::Rgba = rustory_vn_render::Rgba::rgb(238, 240, 246);
    const COL_NAME: rustory_vn_render::Rgba = rustory_vn_render::Rgba::rgb(178, 196, 240);
    const COL_CHOICE: rustory_vn_render::Rgba = rustory_vn_render::Rgba::rgba(26, 31, 46, 248);
    const COL_EDGE: rustory_vn_render::Rgba = rustory_vn_render::Rgba::rgba(120, 150, 210, 200);

    let mut frame = Frame::new(WIDTH, HEIGHT).with_clear(rustory_vn_render::Rgba::rgb(12, 14, 20));

    if let Some(background) = narrative.stage().background()
        && let Some(id) = bank.resolve(background)
    {
        frame.push_quad(Quad::textured(
            Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
            id,
            rustory_vn_render::Rgba::rgb(255, 255, 255),
        ));
    }

    for state in narrative.stage().actors().values() {
        let Some(id) = bank.resolve(&state.pose) else {
            continue;
        };
        let height = 620.0 * SPRITE_SCALE;
        let width = 420.0 * SPRITE_SCALE;
        let x = match state.slot {
            Slot::Left => 80.0,
            Slot::Center => (WIDTH as f32 - width) / 2.0,
            Slot::Right => WIDTH as f32 - 80.0 - width,
        };
        frame.push_quad(Quad::textured(
            Rect::new(x, DIALOGUE.y - height, width, height),
            id,
            rustory_vn_render::Rgba::rgb(255, 255, 255),
        ));
    }

    if let Some(line) = narrative.line() {
        frame.push_quad(Quad::solid(DIALOGUE, COL_BOX));
        frame.push_quad(Quad::solid(
            Rect::new(DIALOGUE.x, DIALOGUE.y, DIALOGUE.width, 2.0),
            COL_EDGE,
        ));
        if let Some(speaker) = line.speaker {
            frame.push_text(
                TextItem::new(speaker.as_str(), 92.0, 522.0, 24.0)
                    .with_color(COL_NAME)
                    .with_wrap(DIALOGUE.width - 64.0),
            );
        }
        frame.push_text(
            TextItem::new(line.visible.to_owned(), 92.0, 566.0, 30.0)
                .with_color(COL_TEXT)
                .with_wrap(DIALOGUE.width - 64.0),
        );
    }

    if let Mode::AwaitingChoice { prompt, choices } = narrative.mode() {
        if let Some(prompt) = prompt {
            let plate = Rect::new(330.0, 108.0, 620.0, 56.0);
            frame.push_quad(Quad::solid(plate, COL_CHOICE));
            frame.push_text(
                TextItem::new(prompt.clone(), plate.x + 24.0, plate.y + 14.0, 26.0)
                    .with_color(COL_TEXT)
                    .with_wrap(plate.width - 48.0),
            );
        }

        let x = (WIDTH as f32 - CHOICE_WIDTH) / 2.0;
        for (index, choice) in choices.iter().enumerate() {
            let y = CHOICE_TOP + index as f32 * (CHOICE_HEIGHT + CHOICE_GAP);
            let rect = Rect::new(x, y, CHOICE_WIDTH, CHOICE_HEIGHT);
            frame.push_quad(Quad::solid(rect, COL_CHOICE));
            frame.push_quad(Quad::solid(
                Rect::new(rect.x, rect.y, 4.0, rect.height),
                COL_EDGE,
            ));
            frame.push_text(
                TextItem::new(
                    format!("{}. {}", index + 1, choice.label),
                    rect.x + 24.0,
                    rect.y + 18.0,
                    26.0,
                )
                .with_color(COL_TEXT)
                .with_wrap(rect.width - 48.0),
            );
        }
    }

    if let Some(prompt) = prompt {
        let plate = Rect::new(DIALOGUE.x, DIALOGUE.y, DIALOGUE.width, DIALOGUE.height);
        frame.push_quad(Quad::solid(plate, COL_CHOICE));
        frame.push_quad(Quad::solid(
            Rect::new(plate.x, plate.y, plate.width, 2.0),
            COL_EDGE,
        ));
        frame.push_text(
            TextItem::new(prompt, plate.x + 32.0, plate.y + 28.0, 28.0)
                .with_color(COL_TEXT)
                .with_wrap(plate.width - 64.0),
        );
    }

    frame
}

/// Texture handles by name.
#[derive(Default, Clone)]
pub struct TextureBank {
    textures: std::collections::HashMap<String, rustory_vn_render::TextureId>,
}

impl TextureBank {
    /// How many distinct images are available.
    ///
    /// The map holds each image under two keys (name and name-with-extension), so this
    /// counts unique handles rather than entries.
    #[must_use]
    pub fn len(&self) -> usize {
        let mut unique: Vec<u32> = self.textures.values().map(|id| id.0).collect();
        unique.sort_unstable();
        unique.dedup();
        unique.len()
    }

    pub fn resolve(&self, name: &str) -> Option<rustory_vn_render::TextureId> {
        self.textures.get(name).copied().or_else(|| {
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            self.textures.get(stem).copied()
        })
    }
}

/// Where the preview looks for art.
///
/// A search path, not a single guess. Projects lay their files out differently: art
/// next to the story, art one level up in a shared `assets/`, or art relative to where
/// the editor was launched. Picking one and being wrong means the preview silently
/// loses every image, which looks exactly like a renderer bug — so the first candidate
/// that actually contains images wins.
#[derive(Debug, Clone, Default)]
pub struct AssetPaths {
    candidates: Vec<std::path::PathBuf>,
    /// The one that was used, once resolved.
    resolved: std::cell::RefCell<Option<std::path::PathBuf>>,
}

impl AssetPaths {
    /// Build the search path from the story's location.
    #[must_use]
    pub fn beside(story: &std::path::Path) -> Self {
        let dir = story.parent().unwrap_or_else(|| std::path::Path::new("."));
        Self {
            candidates: vec![
                dir.join("assets"),
                dir.join("..").join("assets"),
                std::path::PathBuf::from("assets"),
            ],
            resolved: std::cell::RefCell::new(None),
        }
    }

    /// Use the working directory only.
    #[must_use]
    pub fn from_cwd() -> Self {
        Self {
            candidates: vec![std::path::PathBuf::from("assets")],
            resolved: std::cell::RefCell::new(None),
        }
    }

    /// The directory that holds the art, if any candidate does.
    #[must_use]
    pub fn resolve(&self) -> Option<std::path::PathBuf> {
        if let Some(known) = self.resolved.borrow().as_ref() {
            return Some(known.clone());
        }
        for candidate in &self.candidates {
            if contains_png(candidate) {
                *self.resolved.borrow_mut() = Some(candidate.clone());
                return Some(candidate.clone());
            }
        }
        None
    }

    /// Every candidate, for a diagnostic message.
    #[must_use]
    pub fn candidates(&self) -> &[std::path::PathBuf] {
        &self.candidates
    }
}

fn contains_png(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("png"))
}

/// Upload whatever PNGs live in the resolved asset directory.
///
/// Missing assets are not an error: the preview shows the dialogue box over the clear
/// colour, which is exactly what `rustory shot` would produce for the same story. But
/// the number of images found is reported so a blank preview is diagnosable.
pub fn textures(renderer: &mut Renderer, paths: &AssetPaths) -> (TextureBank, usize) {
    let mut bank = TextureBank::default();
    let mut loaded = 0usize;

    let Some(dir) = paths.resolve() else {
        return (bank, 0);
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (bank, 0);
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Ok(image) = Rgba8Image::load_png(&path) else {
            continue;
        };
        let id = renderer.upload_texture(&image, stem);
        bank.textures.insert(stem.to_owned(), id);
        bank.textures.insert(format!("{stem}.png"), id);
        loaded += 1;
    }

    (bank, loaded)
}
