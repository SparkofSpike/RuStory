//! The draw list.
//!
//! The renderer consumes a [`Frame`] and nothing else. It has no idea a story
//! exists: the application translates narrative state into quads and text runs,
//! and this crate turns those into pixels.
//!
//! That split matters for two reasons. It keeps game semantics out of the
//! renderer, and it makes layout testable without a GPU — a frame is ordinary
//! data, so a test can assert that a dialogue box sits where it should.

/// Straight RGBA, 0..=255.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    // Shares its name with the type on purpose: `Rgba::rgba(..)` reads better at
    // call sites than a disambiguated alternative would.
    #[allow(clippy::self_named_constructors)]
    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[must_use]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// Normalized `[r, g, b, a]` as the GPU expects.
    ///
    /// These are **sRGB** values, matching how a designer thinks and how an image file
    /// stores them. The renderer converts to linear at the boundary — see
    /// [`Rgba::to_linear_array`] for why that conversion is required rather than
    /// optional.
    #[must_use]
    pub fn to_array(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }

    /// Normalized `[r, g, b, a]` converted to **linear** light.
    ///
    /// The render target is `Bgra8UnormSrgb`, so wgpu treats every value written to it
    /// as linear and encodes it to sRGB on the way out. Handing it sRGB values means
    /// encoding twice: `12` becomes `61`, and a near-black dialogue box renders as mid
    /// grey. Caught by `tests/colour.rs`, which asserts what actually lands in the
    /// framebuffer rather than what was asked for.
    ///
    /// Alpha is not gamma-encoded, so it passes through untouched.
    #[must_use]
    pub fn to_linear_array(self) -> [f32; 4] {
        [
            srgb_to_linear(self.r),
            srgb_to_linear(self.g),
            srgb_to_linear(self.b),
            self.a as f32 / 255.0,
        ]
    }

    /// The same colour as linear-light `f64`, which is what wgpu's clear value takes.
    #[must_use]
    pub fn to_linear_clear(self) -> [f64; 4] {
        let linear = self.to_linear_array();
        [
            linear[0] as f64,
            linear[1] as f64,
            linear[2] as f64,
            linear[3] as f64,
        ]
    }
}

/// sRGB transfer function, inverse of the encoding the GPU applies on output.
///
/// The piecewise form is not an approximation of the `2.2` power curve; it is the
/// actual standard, and the linear segment near black is what keeps shadows from
/// crushing. Using `powf(2.2)` instead would be visibly wrong in the dark end, which
/// is where a night scene lives.
fn srgb_to_linear(channel: u8) -> f32 {
    let c = channel as f32 / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod colour_space {
    use super::*;

    #[test]
    fn srgb_to_linear_matches_the_standard_at_known_points() {
        // Endpoints are exact, by definition.
        assert!((srgb_to_linear(0) - 0.0).abs() < 1e-6);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);

        // 128/255 ≈ 0.502 sRGB is ≈ 0.216 linear. This is the number that makes
        // "mid grey" look like mid grey instead of light grey.
        assert!((srgb_to_linear(128) - 0.2158).abs() < 0.001);

        // 12/255 is in the linear segment; the result must be far darker than the
        // 0.047 input, which is the whole bug this function exists to fix.
        assert!(srgb_to_linear(12) < 0.005);
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    #[test]
    fn alpha_survives_the_conversion_untouched() {
        let colour = Rgba::rgba(12, 14, 20, 128);
        let linear = colour.to_linear_array();
        assert!((linear[3] - 128.0 / 255.0).abs() < 1e-6);
    }
}

/// An axis-aligned rectangle in logical pixels, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    #[must_use]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// Inset by `amount` on every side, never producing a negative size.
    #[must_use]
    pub fn inset(&self, amount: f32) -> Self {
        let width = (self.width - amount * 2.0).max(0.0);
        let height = (self.height - amount * 2.0).max(0.0);
        Self::new(self.x + amount, self.y + amount, width, height)
    }
}

/// Identifies an uploaded texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub u32);

/// Texture coordinates, normalized. Defaults to the whole image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Uv {
    /// `[u0, v0, u1, v1]`
    pub bounds: [f32; 4],
}

impl Default for Uv {
    fn default() -> Self {
        Self {
            bounds: [0.0, 0.0, 1.0, 1.0],
        }
    }
}

/// One textured or solid-coloured rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    pub rect: Rect,
    /// Multiplied with the sampled texel, so a white texture plus this colour is
    /// a solid rectangle. One code path, no shader branch.
    pub color: Rgba,
    /// `None` draws `color` flat.
    pub texture: Option<TextureId>,
    pub uv: Uv,
}

impl Quad {
    #[must_use]
    pub fn solid(rect: Rect, color: Rgba) -> Self {
        Self {
            rect,
            color,
            texture: None,
            uv: Uv::default(),
        }
    }

    #[must_use]
    pub fn textured(rect: Rect, texture: TextureId, color: Rgba) -> Self {
        Self {
            rect,
            color,
            texture: Some(texture),
            uv: Uv::default(),
        }
    }
}

/// Which typeface a text run asks for.
///
/// A visual novel does not pick one font: dialogue, a name plate and a system
/// message want different faces. This is a layout concern, not a game concept, so
/// it lives in the renderer rather than in the story model.
///
/// `Named` is how a creator selects a font they loaded themselves. The engine
/// ships no fonts and has no opinion about which one a project uses; it only
/// needs to be told.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FontChoice {
    /// Whatever the platform offers as its default sans-serif.
    #[default]
    SansSerif,
    Serif,
    Monospace,
    /// A family name, resolved against loaded fonts first and installed fonts
    /// second. Skips to the generic sans-serif if nothing matches, so a typo
    /// costs a wrong typeface rather than an invisible line of dialogue.
    Named(String),
}

impl FontChoice {
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into())
    }
}

/// Horizontal alignment of a text run inside its wrap width.
///
/// Two choices rather than cosmic-text's full set: the engine's own layout uses left-aligned
/// prose and centred headings, and a justified line in a dialogue box looks like a mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
}

/// A run of text to shape and draw.
#[derive(Debug, Clone, PartialEq)]
pub struct TextItem {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub color: Rgba,
    /// Wrap width. Text beyond this wraps rather than overflowing.
    pub max_width: f32,
    /// Which face to shape with.
    pub font: FontChoice,
    /// Where each line starts inside `max_width`.
    pub align: TextAlign,
}

impl TextItem {
    #[must_use]
    pub fn new(text: impl Into<String>, x: f32, y: f32, font_size: f32) -> Self {
        Self {
            text: text.into(),
            x,
            y,
            font_size,
            line_height: font_size * 1.3,
            color: Rgba::rgb(255, 255, 255),
            max_width: f32::INFINITY,
            font: FontChoice::default(),
            align: TextAlign::Left,
        }
    }

    #[must_use]
    pub fn with_font(mut self, font: FontChoice) -> Self {
        self.font = font;
        self
    }

    #[must_use]
    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    #[must_use]
    pub fn with_color(mut self, color: Rgba) -> Self {
        self.color = color;
        self
    }

    #[must_use]
    pub fn with_wrap(mut self, max_width: f32) -> Self {
        self.max_width = max_width;
        self
    }
}

/// Everything to draw for one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Clear colour.
    pub clear: Rgba,
    pub quads: Vec<Quad>,
    pub texts: Vec<TextItem>,
}

impl Frame {
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            clear: Rgba::rgb(0, 0, 0),
            quads: Vec::new(),
            texts: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_clear(mut self, clear: Rgba) -> Self {
        self.clear = clear;
        self
    }

    pub fn push_quad(&mut self, quad: Quad) {
        self.quads.push(quad);
    }

    pub fn push_text(&mut self, text: TextItem) {
        self.texts.push(text);
    }

    /// Total draw primitives, for tests and diagnostics.
    #[must_use]
    pub fn primitive_count(&self) -> usize {
        self.quads.len() + self.texts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_geometry_is_top_left_origin() {
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(rect.right(), 110.0);
        assert_eq!(rect.bottom(), 70.0);
        assert!(rect.contains(10.0, 20.0));
        assert!(rect.contains(109.9, 69.9));
        assert!(!rect.contains(110.0, 20.0), "right edge is exclusive");
        assert!(!rect.contains(10.0, 70.0), "bottom edge is exclusive");
    }

    #[test]
    fn inset_never_goes_negative() {
        let rect = Rect::new(0.0, 0.0, 10.0, 4.0).inset(8.0);
        assert_eq!(rect.width, 0.0);
        assert_eq!(rect.height, 0.0);
    }

    #[test]
    fn colours_normalize_to_unit_range() {
        let c = Rgba::rgb(255, 0, 128);
        let array = c.to_array();
        assert!((array[0] - 1.0).abs() < f32::EPSILON);
        assert!((array[1]).abs() < f32::EPSILON);
        assert!((array[2] - 128.0 / 255.0).abs() < 1e-6);
        assert!((array[3] - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn default_uv_covers_the_whole_texture() {
        assert_eq!(Uv::default().bounds, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn a_frame_counts_its_primitives() {
        let mut frame = Frame::new(1280, 720);
        assert_eq!(
            frame.primitive_count(),
            0,
            "clear colour is not a primitive"
        );
        frame.push_quad(Quad::solid(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rgba::rgb(1, 2, 3),
        ));
        frame.push_text(TextItem::new("hi", 0.0, 0.0, 20.0));
        assert_eq!(frame.primitive_count(), 2);
    }
}
