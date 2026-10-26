//! Procedural placeholder art.
//!
//! The demo needs a background and two sprites. Shipping binary art in the
//! repository would make the first clone heavier and the diff opaque, so the
//! assets are generated: reproducible, diffable as code, and good enough to
//! prove layout works.
//!
//! Run `rustory gen-assets assets` to write them.

use std::path::{Path, PathBuf};

use rustory_render::{ImageError, Rgba, Rgba8Image};

/// A tiny mutable RGBA canvas, so shapes can be drawn with plain arithmetic.
struct Canvas {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width * height * 4) as usize],
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: Rgba) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let offset = ((y as u32 * self.width + x as u32) * 4) as usize;
        let src = [color.r, color.g, color.b, color.a];
        let dst = &mut self.data[offset..offset + 4];
        let alpha = src[3] as f32 / 255.0;
        for channel in 0..3 {
            dst[channel] =
                (src[channel] as f32 * alpha + dst[channel] as f32 * (1.0 - alpha)).round() as u8;
        }
        // Source-over on premultiplied-agnostic straight alpha.
        let dst_alpha = dst[3] as f32 / 255.0;
        dst[3] = ((alpha + dst_alpha * (1.0 - alpha)) * 255.0).round() as u8;
    }

    fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Rgba) {
        for py in y..y + height {
            for px in x..x + width {
                self.blend(px, py, color);
            }
        }
    }

    fn fill_ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, color: Rgba) {
        let x0 = (cx - rx).floor() as i32;
        let x1 = (cx + rx).ceil() as i32;
        let y0 = (cy - ry).floor() as i32;
        let y1 = (cy + ry).ceil() as i32;
        for py in y0..=y1 {
            for px in x0..=x1 {
                let dx = (px as f32 - cx) / rx;
                let dy = (py as f32 - cy) / ry;
                if dx * dx + dy * dy <= 1.0 {
                    self.blend(px, py, color);
                }
            }
        }
    }

    /// Half-width of a torso at height `y`, for a simple tapered body.
    fn torso_half_width(y: f32, top: f32, bottom: f32, shoulder: f32, waist: f32) -> f32 {
        let t = ((y - top) / (bottom - top)).clamp(0.0, 1.0);
        shoulder + (waist - shoulder) * t
    }

    fn vertical_gradient(&mut self, top: Rgba, bottom: Rgba) {
        let last = self.height.saturating_sub(1).max(1) as f32;
        for y in 0..self.height {
            let t = y as f32 / last;
            let lerp = |a: u8, b: u8| {
                (a as f32 + (b as f32 - a as f32) * t)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            let color = Rgba::rgba(
                lerp(top.r, bottom.r),
                lerp(top.g, bottom.g),
                lerp(top.b, bottom.b),
                lerp(top.a, bottom.a),
            );
            self.fill_rect(0, y as i32, self.width as i32, 1, color);
        }
    }

    fn into_image(self) -> Rgba8Image {
        Rgba8Image::from_rgba(self.width, self.height, self.data)
            .expect("canvas is always exactly width * height * 4")
    }
}

/// A classroom at dusk: warm floor, cool wall, a horizon band where they meet.
fn background() -> Rgba8Image {
    let mut canvas = Canvas::new(1280, 720);
    canvas.vertical_gradient(Rgba::rgb(38, 48, 74), Rgba::rgb(28, 32, 48));

    // Wall panel line, so the background is not a flat wash.
    canvas.fill_rect(0, 470, 1280, 6, Rgba::rgb(58, 66, 92));

    // Window light spilling from the left.
    for i in 0..80 {
        let alpha = (40 - i / 2).max(0) as u8;
        canvas.fill_rect(60 + i * 4, 80, 4, 380, Rgba::rgba(120, 150, 210, alpha));
    }

    // Floor.
    canvas.fill_rect(0, 476, 1280, 244, Rgba::rgb(46, 40, 44));
    canvas.fill_rect(0, 476, 1280, 3, Rgba::rgb(90, 76, 70));

    canvas.into_image()
}

/// A stylised character silhouette: head, torso, and a colour accent.
fn sprite(body: Rgba, accent: Rgba) -> Rgba8Image {
    let mut canvas = Canvas::new(420, 620);

    // Torso, tapering from shoulders to waist.
    for y in 180..620 {
        let half = Canvas::torso_half_width(y as f32, 180.0, 620.0, 118.0, 96.0);
        let cx = 210.0;
        canvas.fill_rect(
            (cx - half) as i32,
            y,
            (half * 2.0) as i32,
            1,
            body,
        );
    }

    // Neck.
    canvas.fill_rect(190, 140, 40, 60, body);

    // Head.
    canvas.fill_ellipse(210.0, 100.0, 66.0, 78.0, body);

    // Hair sweep.
    canvas.fill_ellipse(210.0, 62.0, 72.0, 46.0, accent);
    canvas.fill_rect(138, 62, 20, 120, accent);
    canvas.fill_rect(262, 62, 20, 120, accent);

    // Collar accent, so the two sprites are distinguishable at a glance.
    canvas.fill_rect(150, 186, 120, 14, accent);

    canvas.into_image()
}

/// Paths written by [`generate`].
pub fn generate(dir: impl AsRef<Path>) -> Result<Vec<PathBuf>, ImageError> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).map_err(|err| ImageError {
        path: dir.display().to_string(),
        message: err.to_string(),
    })?;

    let files = [
        ("bg_classroom.png", background()),
        (
            "yuki_smile.png",
            sprite(Rgba::rgba(232, 224, 236, 255), Rgba::rgba(120, 96, 160, 255)),
        ),
        (
            "kai_neutral.png",
            sprite(Rgba::rgba(210, 218, 226, 255), Rgba::rgba(64, 92, 120, 255)),
        ),
    ];

    let mut written = Vec::new();
    for (name, image) in files {
        let path = dir.join(name);
        image.save_png(&path)?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_assets_have_the_expected_dimensions() {
        let bg = background();
        assert_eq!((bg.width(), bg.height()), (1280, 720));

        let character = sprite(Rgba::rgb(255, 255, 255), Rgba::rgb(0, 0, 0));
        assert_eq!((character.width(), character.height()), (420, 620));
    }

    #[test]
    fn a_sprite_is_transparent_above_the_head() {
        let character = sprite(Rgba::rgb(240, 240, 240), Rgba::rgb(80, 80, 80));
        let corner = character.pixel(0, 0).expect("in range");
        assert_eq!(
            corner.a, 0,
            "the sprite must have a transparent background so it composites over the scene"
        );
    }

    #[test]
    fn a_sprite_is_opaque_through_the_torso() {
        let character = sprite(Rgba::rgb(240, 240, 240), Rgba::rgb(80, 80, 80));
        let chest = character.pixel(210, 300).expect("in range");
        assert_eq!(chest.a, 255);
    }

    #[test]
    fn generation_writes_every_file() {
        let dir = std::env::temp_dir().join("rustory-assets-test");
        let _ = std::fs::remove_dir_all(&dir);

        let written = generate(&dir).expect("generate");
        assert_eq!(written.len(), 3);
        for path in &written {
            assert!(path.exists(), "{} should exist", path.display());
            assert!(path.metadata().expect("stat").len() > 0);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
