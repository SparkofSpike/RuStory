//! `--check`: render the preview without a window.
//!
//! A GUI that can only be inspected by a human looking at it is a GUI nobody can
//! regression-test. This mode renders exactly what the preview panel would show, into
//! a PNG, and asserts the result is not blank — which is the failure mode a preview
//! actually has. An empty panel looks like a styling problem; it is almost always a
//! resource that never bound.

use std::path::Path;

use rustory_vn_core::{Mode, Narrative, Story};
use rustory_vn_render::{Renderer, Rgba8Image};

use crate::preview;

/// Render the story at `step` and report what came out.
pub fn run(story: &Story, step: usize, out: Option<&Path>) -> Result<Report, String> {
    let mut renderer = Renderer::new_headless(preview::WIDTH, preview::HEIGHT)
        .map_err(|err| format!("no GPU for the preview check: {err}"))?;

    let mut narrative = Narrative::new(story.clone()).map_err(|err| err.to_string())?;
    let mut registry = preview::default_registry();

    for _ in 0..step {
        if matches!(narrative.mode(), Mode::Finished { .. }) {
            break;
        }
        preview::step(&mut narrative, &mut registry)?;
    }
    narrative.tick(0, 0);
    if matches!(narrative.mode(), Mode::Suspended { .. }) {
        let _ = narrative.step_gameplay(&mut registry, 0, &[]);
    }

    // Beside the story, matching what the GUI does.
    let paths = preview::AssetPaths::from_cwd();
    let (bank, _images) = preview::textures(&mut renderer, &paths);

    let prompt = narrative.gameplay_prompt(&registry);
    let frame = preview::build_frame(&narrative, &bank, prompt.as_deref());

    let image = renderer
        .render_offscreen(&frame)
        .map_err(|err| err.to_string())?;

    let stats = analyse(&image);
    if let Some(path) = out {
        image.save_png(path).map_err(|err| err.to_string())?;
    }

    Ok(Report {
        node: narrative.cursor().to_string(),
        primitives: frame.primitive_count(),
        unique_colours: stats.unique_colours,
        bright_pixels: stats.bright_pixels,
        out: out.map(|path| path.display().to_string()),
    })
}

#[derive(Debug)]
pub struct Report {
    pub node: String,
    pub primitives: usize,
    pub unique_colours: usize,
    pub bright_pixels: usize,
    pub out: Option<String>,
}

impl Report {
    /// Whether the image looks like a rendered scene rather than a flat wash.
    ///
    /// The threshold is deliberately crude. A frame with a background, a dialogue box
    /// and text has many colours and a good number of bright pixels; a failed bind
    /// produces one colour and almost none. Anything in between is a real question for
    /// a human, not something to fail on.
    #[must_use]
    pub fn looks_rendered(&self) -> bool {
        self.unique_colours >= 8 && self.bright_pixels >= 200
    }
}

#[derive(Debug)]
struct Stats {
    unique_colours: usize,
    bright_pixels: usize,
}

fn analyse(image: &Rgba8Image) -> Stats {
    use std::collections::BTreeSet;

    let mut colours = BTreeSet::new();
    let mut bright = 0usize;

    // Sampling every fourth pixel: enough to characterise the frame, fast enough not
    // to matter on a 1280x720 image.
    for y in (0..image.height()).step_by(2) {
        for x in (0..image.width()).step_by(2) {
            let Some(pixel) = image.pixel(x, y) else {
                continue;
            };
            colours.insert((pixel.r, pixel.g, pixel.b));
            if pixel.r as u32 + pixel.g as u32 + pixel.b as u32 > 200 {
                bright += 1;
            }
        }
    }

    Stats {
        unique_colours: colours.len(),
        bright_pixels: bright,
    }
}
