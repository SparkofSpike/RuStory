//! What a gameplay system draws while it has control.
//!
//! A system used to be able to say one thing: a block of text. That was enough for a countdown
//! and a quiz, and not enough for anything with a board, a cursor, a drag target or a bar. This
//! module is the channel for those, kept deliberately narrow:
//!
//! - It is **data**, not draw calls. A [`GameplayView`] is a list of elements with numbers in
//!   it, so a test can assert where a bar sits without a GPU.
//! - It is **layout-free in pixels and fixed in space**: coordinates live in a 1280×720 design
//!   box, the same one `rustory-vn-render`'s reference frame and the app's scene use. A system
//!   does not learn the window size, because a window that resizes must not change the rules.
//! - It is **not** a UI toolkit. Four element kinds, colours by role rather than by value. A
//!   system that needs a widget gets a review of this list first — that review is the point.
//!
//! # What this does not do
//!
//! It does not let a system place a raw pixel, sample its own shader or load a font. Those are
//! decisions that belong to the project, not to a gameplay rule, and a system that could make
//! them could no longer be swapped into another project's visual style.
//!
//! # Example
//!
//! ```rust
//! use rustory_vn_core::{Element, GameplayView, Tone, ViewRect};
//!
//! let view = GameplayView::new()
//!     .push(Element::panel(ViewRect::full(), Tone::Panel))
//!     .push(Element::text(
//!         "Round 2 of 3",
//!         ViewRect::new(60.0, 60.0, 600.0, 40.0),
//!         32.0,
//!         Tone::Text,
//!     ))
//!     .push(Element::bar(
//!         ViewRect::new(60.0, 640.0, 1160.0, 12.0),
//!         0.5,
//!         Tone::Accent,
//!     ));
//!
//! assert_eq!(view.elements.len(), 3);
//! ```

/// The design width every gameplay view is laid out in.
pub const VIEW_WIDTH: f32 = 1280.0;

/// The design height every gameplay view is laid out in.
pub const VIEW_HEIGHT: f32 = 720.0;

/// A rectangle in the design box, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ViewRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ViewRect {
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The whole design box. A full-screen backdrop is the common case for a gameplay scene:
    /// the story behind it is not what the player is deciding about.
    #[must_use]
    pub const fn full() -> Self {
        Self::new(0.0, 0.0, VIEW_WIDTH, VIEW_HEIGHT)
    }

    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Whether the rectangle covers any area at all.
    ///
    /// A zero-sized element draws nothing, so a system may build one speculatively and let the
    /// renderer skip it.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.width > 0.0 && self.height > 0.0 && self.x.is_finite() && self.y.is_finite()
    }

    /// Inset on every side, never producing a negative size.
    #[must_use]
    pub fn inset(&self, amount: f32) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.width - amount * 2.0).max(0.0),
            (self.height - amount * 2.0).max(0.0),
        )
    }
}

/// What a colour means.
///
/// A system names a role; the project decides the value. That is what keeps a gameplay system
/// from carrying its own visual identity into a story that has one already, and it is why a
/// system can be moved between projects without a restyle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Tone {
    /// A surface behind content: a full-screen backdrop, a card.
    Panel,
    /// A surface that sits on top of a [`Tone::Panel`]: a row, a button.
    Raised,
    /// Body text.
    Text,
    /// Supporting text: a countdown, a hint, a score.
    Muted,
    /// The thing the system wants the player to look at.
    Accent,
    /// Something going wrong: a wrong answer, a nearly expired timer.
    Danger,
    /// Something going right.
    Success,
    /// The border of a surface that is not [`Tone::Raised`].
    Edge,
}

/// Where a text run starts inside its rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Align {
    /// Left edge, which is what prose wants.
    #[default]
    Start,
    /// Centred, which is what a title and a countdown want.
    Center,
}

/// One thing to draw.
///
/// Elements are drawn in list order, so a later element covers an earlier one. That is the whole
/// of the layering model — a system that needs z-order reorders its list.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Element {
    /// A wrapped run of text.
    ///
    /// `rect.width` is the wrap width; `rect.height` is advisory (the renderer does not clip).
    /// `size` is in design pixels.
    Text {
        text: String,
        rect: ViewRect,
        size: f32,
        tone: Tone,
        align: Align,
    },
    /// A filled rectangle, optionally outlined one design pixel wide per side.
    Panel {
        rect: ViewRect,
        tone: Tone,
        border: Option<Tone>,
    },
    /// An image from the project's asset bank, stretched into `rect`.
    ///
    /// Missing art is skipped rather than drawn as a placeholder — the same rule sprites follow.
    Image { name: String, rect: ViewRect },
    /// A horizontal progress bar: an empty track with a filled portion.
    ///
    /// `fraction` is clamped to `0..=1` when the element is built, so a countdown that overshoots
    /// by a millisecond cannot draw outside its own track.
    Bar {
        rect: ViewRect,
        fraction: f32,
        tone: Tone,
    },
}

impl Element {
    /// A text run with left alignment.
    #[must_use]
    pub fn text(text: impl Into<String>, rect: ViewRect, size: f32, tone: Tone) -> Self {
        Self::Text {
            text: text.into(),
            rect,
            size,
            tone,
            align: Align::Start,
        }
    }

    /// The same run, centred.
    #[must_use]
    pub fn centered_text(text: impl Into<String>, rect: ViewRect, size: f32, tone: Tone) -> Self {
        Self::Text {
            text: text.into(),
            rect,
            size,
            tone,
            align: Align::Center,
        }
    }

    /// A filled rectangle with no outline.
    #[must_use]
    pub fn panel(rect: ViewRect, tone: Tone) -> Self {
        Self::Panel {
            rect,
            tone,
            border: None,
        }
    }

    /// A filled rectangle with an outline.
    #[must_use]
    pub fn outlined(rect: ViewRect, tone: Tone, border: Tone) -> Self {
        Self::Panel {
            rect,
            tone,
            border: Some(border),
        }
    }

    /// An image, resolved by name against the project's textures.
    #[must_use]
    pub fn image(name: impl Into<String>, rect: ViewRect) -> Self {
        Self::Image {
            name: name.into(),
            rect,
        }
    }

    /// A progress bar. `fraction` is clamped into `0..=1`; a non-finite value reads as zero.
    #[must_use]
    pub fn bar(rect: ViewRect, fraction: f32, tone: Tone) -> Self {
        let fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        Self::Bar {
            rect,
            fraction,
            tone,
        }
    }

    /// The rectangle the element occupies, for tests and hit-testing.
    #[must_use]
    pub fn rect(&self) -> ViewRect {
        match self {
            Self::Text { rect, .. }
            | Self::Panel { rect, .. }
            | Self::Image { rect, .. }
            | Self::Bar { rect, .. } => *rect,
        }
    }
}

/// Everything a gameplay system draws for one frame.
///
/// Empty is legal and means "draw nothing of my own", which is what a headless run or a system
/// that only needs the story's own screen wants.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct GameplayView {
    pub elements: Vec<Element>,
}

impl GameplayView {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            elements: Vec::with_capacity(capacity),
        }
    }

    /// Appends one element and returns the view, so a system can build its screen in one
    /// expression.
    #[must_use]
    pub fn push(mut self, element: Element) -> Self {
        self.elements.push(element);
        self
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Every text run, in draw order. Used by tests and by the transcript fallback.
    pub fn texts(&self) -> impl Iterator<Item = &str> {
        self.elements.iter().filter_map(|element| match element {
            Element::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
    }

    /// Every progress bar, in draw order.
    pub fn bars(&self) -> impl Iterator<Item = (ViewRect, f32)> + '_ {
        self.elements.iter().filter_map(|element| match element {
            Element::Bar { rect, fraction, .. } => Some((*rect, *fraction)),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_view_draws_nothing() {
        let view = GameplayView::new();
        assert!(view.is_empty());
        assert_eq!(view.len(), 0);
        assert_eq!(view.texts().count(), 0);
    }

    #[test]
    fn elements_keep_the_order_they_were_pushed_in() {
        let view = GameplayView::new()
            .push(Element::panel(ViewRect::full(), Tone::Panel))
            .push(Element::text(
                "Round 1",
                ViewRect::new(0.0, 0.0, 100.0, 20.0),
                24.0,
                Tone::Text,
            ));

        assert_eq!(view.len(), 2);
        assert_eq!(view.texts().collect::<Vec<_>>(), ["Round 1"]);
        // The panel is first because it was pushed first, which is why it sits behind the text.
        assert!(matches!(view.elements[0], Element::Panel { .. }));
    }

    #[test]
    fn a_bar_fraction_is_clamped_into_its_track() {
        let rect = ViewRect::new(0.0, 0.0, 100.0, 10.0);

        let over = Element::bar(rect, 4.2, Tone::Accent);
        let under = Element::bar(rect, -0.3, Tone::Accent);
        let nan = Element::bar(rect, f32::NAN, Tone::Accent);

        assert!(matches!(over, Element::Bar { fraction: 1.0, .. }));
        assert!(matches!(under, Element::Bar { fraction: 0.0, .. }));
        assert!(
            matches!(nan, Element::Bar { fraction: 0.0, .. }),
            "a NaN fraction must not reach the renderer as a NaN coordinate"
        );
    }

    #[test]
    fn bars_report_their_own_fraction() {
        let view = GameplayView::new()
            .push(Element::bar(
                ViewRect::new(0.0, 0.0, 100.0, 8.0),
                0.25,
                Tone::Accent,
            ))
            .push(Element::text(
                "time",
                ViewRect::new(0.0, 0.0, 50.0, 10.0),
                12.0,
                Tone::Muted,
            ));

        let bars: Vec<_> = view.bars().collect();
        assert_eq!(bars.len(), 1);
        assert!((bars[0].1 - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn a_rect_knows_whether_it_covers_anything() {
        assert!(ViewRect::full().is_visible());
        assert!(!ViewRect::new(10.0, 10.0, 0.0, 30.0).is_visible());
        assert!(!ViewRect::new(f32::NAN, 10.0, 30.0, 30.0).is_visible());
    }

    #[test]
    fn insetting_never_goes_negative() {
        let rect = ViewRect::new(0.0, 0.0, 10.0, 4.0).inset(8.0);
        assert_eq!(rect.width, 0.0);
        assert_eq!(rect.height, 0.0);
    }

    #[test]
    fn full_covers_the_design_box() {
        let rect = ViewRect::full();
        assert_eq!(rect.right(), VIEW_WIDTH);
        assert_eq!(rect.bottom(), VIEW_HEIGHT);
    }
}
