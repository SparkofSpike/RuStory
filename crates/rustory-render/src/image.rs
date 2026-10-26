//! CPU-side images: decoding textures, and writing rendered frames out as PNG.
//!
//! The PNG writer exists so a frame can be inspected as a file instead of as a
//! window nobody can screenshot in CI. "Does it actually draw?" becomes a
//! question with an answer.

use std::path::Path;

use crate::frame::Rgba;

/// An 8-bit RGBA image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba8Image {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Rgba8Image {
    /// Wrap existing RGBA bytes.
    ///
    /// Returns `None` if `data` is not exactly `width * height * 4` bytes, so a
    /// mismatched buffer cannot silently produce a skewed image.
    #[must_use]
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if data.len() != expected {
            return None;
        }
        Some(Self {
            width,
            height,
            data,
        })
    }

    /// A flat image, useful for placeholder textures and test fixtures.
    #[must_use]
    pub fn solid(width: u32, height: u32, color: Rgba) -> Self {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            data.extend_from_slice(&[color.r, color.g, color.b, color.a]);
        }
        Self {
            width,
            height,
            data,
        }
    }

    /// A vertical two-colour gradient. Used by the demo so a background looks
    /// like a background rather than a flat wall.
    #[must_use]
    pub fn vertical_gradient(width: u32, height: u32, top: Rgba, bottom: Rgba) -> Self {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        let last_row = height.saturating_sub(1).max(1);
        for y in 0..height {
            let t = y as f32 / last_row as f32;
            let lerp = |a: u8, b: u8| {
                (a as f32 + (b as f32 - a as f32) * t)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            for _ in 0..width {
                data.extend_from_slice(&[
                    lerp(top.r, bottom.r),
                    lerp(top.g, bottom.g),
                    lerp(top.b, bottom.b),
                    lerp(top.a, bottom.a),
                ]);
            }
        }
        Self {
            width,
            height,
            data,
        }
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgba> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = ((y * self.width + x) * 4) as usize;
        Some(Rgba::rgba(
            self.data[offset],
            self.data[offset + 1],
            self.data[offset + 2],
            self.data[offset + 3],
        ))
    }

    /// Decode a PNG file.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, ImageError> {
        let path = path.as_ref();
        let image = image::open(path).map_err(|err| ImageError {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        let rgba = image.to_rgba8();
        let (width, height) = (rgba.width(), rgba.height());
        Ok(Self {
            width,
            height,
            data: rgba.into_raw(),
        })
    }

    /// Write a PNG.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), ImageError> {
        let path = path.as_ref();
        let file = std::fs::File::create(path).map_err(|err| ImageError {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        let writer = std::io::BufWriter::new(file);
        let mut encoder = png::Encoder::new(writer, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|err| ImageError {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        writer
            .write_image_data(&self.data)
            .map_err(|err| ImageError {
                path: path.display().to_string(),
                message: err.to_string(),
            })?;
        Ok(())
    }
}

/// A texture could not be loaded or written.
#[derive(Debug)]
pub struct ImageError {
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for ImageError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_rgba_rejects_a_buffer_of_the_wrong_size() {
        assert!(Rgba8Image::from_rgba(2, 2, vec![0; 16]).is_some());
        assert!(Rgba8Image::from_rgba(2, 2, vec![0; 15]).is_none());
    }

    #[test]
    fn solid_fills_every_pixel() {
        let image = Rgba8Image::solid(3, 2, Rgba::rgb(10, 20, 30));
        assert_eq!(image.width(), 3);
        assert_eq!(image.height(), 2);
        for y in 0..2 {
            for x in 0..3 {
                assert_eq!(image.pixel(x, y), Some(Rgba::rgb(10, 20, 30)));
            }
        }
    }

    #[test]
    fn gradient_runs_top_to_bottom() {
        let image =
            Rgba8Image::vertical_gradient(1, 3, Rgba::rgb(0, 0, 0), Rgba::rgb(255, 255, 255));
        assert_eq!(image.pixel(0, 0).unwrap().r, 0);
        assert_eq!(image.pixel(0, 2).unwrap().r, 255);
        assert_eq!(image.pixel(0, 1).unwrap().r, 128);
    }

    #[test]
    fn a_png_round_trips_through_disk() {
        let dir = std::env::temp_dir().join("rustory-render-test");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("round-trip.png");

        let original = Rgba8Image::solid(4, 3, Rgba::rgba(12, 34, 56, 200));
        original.save_png(&path).expect("save");
        let loaded = Rgba8Image::load_png(&path).expect("load");

        assert_eq!(loaded.width(), 4);
        assert_eq!(loaded.height(), 3);
        assert_eq!(loaded.pixel(0, 0), Some(Rgba::rgba(12, 34, 56, 200)));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn loading_a_missing_file_reports_the_path() {
        let err = Rgba8Image::load_png("definitely/not/here.png").expect_err("must fail");
        assert!(err.to_string().contains("definitely/not/here.png"));
    }
}
