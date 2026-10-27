//! Turning a gameplay view into a draw list.
//!
//! This lives in the renderer rather than in each front-end because there are two front-ends
//! (the player and the editor's preview) and a third is a matter of time. The editor already
//! duplicates the *story* layout because hoisting story concepts into the renderer would be
//! wrong; a gameplay view is not a story concept — it is a handful of rectangles and text runs —
//! so it can be shared without the same objection.
//!
//! What stays outside: an asset bank. The renderer has no idea what a project's texture names
//! mean, so it asks through [`TextureResolver`].

use rustory_vn_core::{Align, Element, GameplayView, Tone, ViewRect};

use crate::frame::{Frame, Quad, Rect, Rgba, TextAlign, TextItem, TextureId};

/// Resolves an asset name — a name a gameplay system wrote — to an uploaded texture.
///
/// Implemented for any `Fn(&str) -> Option<TextureId>`, so a caller passes a closure over its
/// own bank. `None` means "no such art", which draws nothing.
pub trait TextureResolver {
    fn resolve(&self, name: &str) -> Option<TextureId>;
}

impl<F> TextureResolver for F
where
    F: Fn(&str) -> Option<TextureId>,
{
    fn resolve(&self, name: &str) -> Option<TextureId> {
        self(name)
    }
}

/// The colours a gameplay view is drawn with.
///
/// A system names a [`Tone`]; the values are the project's. The defaults match the engine's own
/// dark scene, and a project with a different look overrides the fields it cares about rather
/// than asking every system to carry colours.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub panel: Rgba,
    pub raised: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub edge: Rgba,
    /// The empty part of a bar. Low-alpha white, so it sits on any backdrop.
    pub bar_track: Rgba,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            // Near-opaque on purpose: a gameplay screen is a screen, not a caption. The story
            // Opaque, and that is a measured decision rather than a taste one. Blending happens
            // in linear light, so a nominally-similar 252 alpha lets 8% of a sprite through and
            // a face reads clearly through the title. A full-screen backdrop is a screen, not a
            // caption; a system that wants the story behind it to show uses `Tone::Raised` on a
            // smaller rectangle instead.
            panel: Rgba::rgba(6, 8, 12, 255),
            raised: Rgba::rgba(26, 31, 46, 248),
            text: Rgba::rgb(238, 240, 246),
            muted: Rgba::rgba(200, 210, 230, 200),
            accent: Rgba::rgb(120, 180, 255),
            danger: Rgba::rgb(240, 110, 110),
            success: Rgba::rgb(130, 215, 160),
            edge: Rgba::rgba(120, 150, 210, 200),
            bar_track: Rgba::rgba(255, 255, 255, 26),
        }
    }
}

impl Palette {
    #[must_use]
    pub fn colour(&self, tone: Tone) -> Rgba {
        match tone {
            Tone::Panel => self.panel,
            Tone::Raised => self.raised,
            Tone::Text => self.text,
            Tone::Muted => self.muted,
            Tone::Accent => self.accent,
            Tone::Danger => self.danger,
            Tone::Success => self.success,
            Tone::Edge => self.edge,
        }
    }
}

/// Draws every element of `view` into `frame`, in list order.
///
/// Elements that cover no area, carry no text or name art the project does not have are skipped
/// silently. That is deliberate: a system should be able to build a screen speculatively, and a
/// missing sprite in a story is already treated the same way.
pub fn paint_gameplay_view(
    view: &GameplayView,
    palette: &Palette,
    textures: &dyn TextureResolver,
    frame: &mut Frame,
) {
    for element in &view.elements {
        match element {
            Element::Panel { rect, tone, border } => {
                if !rect.is_visible() {
                    continue;
                }
                let quad = to_rect(*rect);
                frame.push_quad(Quad::solid(quad, palette.colour(*tone)));
                if let Some(border) = border {
                    push_border(frame, quad, palette.colour(*border));
                }
            }
            Element::Text {
                text,
                rect,
                size,
                tone,
                align,
            } => {
                if text.is_empty() || *size <= 0.0 || !rect.width.is_finite() {
                    continue;
                }
                let item = TextItem::new(text.clone(), rect.x, rect.y, *size)
                    .with_color(palette.colour(*tone))
                    .with_wrap(rect.width.max(1.0))
                    .with_align(match align {
                        Align::Start => TextAlign::Left,
                        Align::Center => TextAlign::Center,
                    });
                frame.push_text(item);
            }
            Element::Image { name, rect } => {
                if !rect.is_visible() {
                    continue;
                }
                let Some(id) = textures.resolve(name) else {
                    continue;
                };
                frame.push_quad(Quad::textured(to_rect(*rect), id, Rgba::rgb(255, 255, 255)));
            }
            Element::Bar {
                rect,
                fraction,
                tone,
            } => {
                if !rect.is_visible() {
                    continue;
                }
                let track = to_rect(*rect);
                frame.push_quad(Quad::solid(track, palette.bar_track));
                let filled = track.width * fraction;
                if filled > 0.0 {
                    frame.push_quad(Quad::solid(
                        Rect::new(track.x, track.y, filled, track.height),
                        palette.colour(*tone),
                    ));
                }
            }
        }
    }
}

fn to_rect(rect: ViewRect) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, rect.height)
}

/// Draws an outline as four thin quads.
///
/// The renderer has no stroke primitive, and adding a second pipeline for four edges would cost
/// more than it saves. Two design pixels is the same weight the dialogue box's rule uses.
fn push_border(frame: &mut Frame, rect: Rect, colour: Rgba) {
    const EDGE: f32 = 2.0;
    frame.push_quad(Quad::solid(
        Rect::new(rect.x, rect.y, rect.width, EDGE),
        colour,
    ));
    frame.push_quad(Quad::solid(
        Rect::new(rect.x, rect.bottom() - EDGE, rect.width, EDGE),
        colour,
    ));
    frame.push_quad(Quad::solid(
        Rect::new(rect.x, rect.y, EDGE, rect.height),
        colour,
    ));
    frame.push_quad(Quad::solid(
        Rect::new(rect.right() - EDGE, rect.y, EDGE, rect.height),
        colour,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustory_vn_core::{Element, GameplayView, Tone, ViewRect};

    fn frame() -> Frame {
        Frame::new(1280, 720)
    }

    fn no_art(_: &str) -> Option<TextureId> {
        None
    }

    #[test]
    fn a_panel_becomes_one_quad_and_an_outline_becomes_four_more() {
        let view = GameplayView::new()
            .push(Element::panel(ViewRect::full(), Tone::Panel))
            .push(Element::outlined(
                ViewRect::new(10.0, 10.0, 100.0, 40.0),
                Tone::Raised,
                Tone::Edge,
            ));

        let mut f = frame();
        paint_gameplay_view(&view, &Palette::default(), &no_art, &mut f);

        assert_eq!(
            f.quads.len(),
            6,
            "one solid panel, then a body plus four edges"
        );
        assert_eq!(f.texts.len(), 0);
    }

    #[test]
    fn a_bar_draws_its_track_and_only_as_much_fill_as_it_has() {
        let view = GameplayView::new().push(Element::bar(
            ViewRect::new(100.0, 600.0, 400.0, 12.0),
            0.25,
            Tone::Accent,
        ));

        let mut f = frame();
        paint_gameplay_view(&view, &Palette::default(), &no_art, &mut f);

        assert_eq!(f.quads.len(), 2, "track plus fill");
        let fill = &f.quads[1];
        assert!((fill.rect.width - 100.0).abs() < 0.01);
        assert_eq!(fill.color, Palette::default().accent);
    }

    #[test]
    fn an_empty_bar_draws_only_the_track() {
        let view = GameplayView::new().push(Element::bar(
            ViewRect::new(0.0, 0.0, 100.0, 8.0),
            0.0,
            Tone::Danger,
        ));

        let mut f = frame();
        paint_gameplay_view(&view, &Palette::default(), &no_art, &mut f);

        assert_eq!(f.quads.len(), 1);
    }

    #[test]
    fn text_keeps_its_size_colour_and_alignment() {
        let view = GameplayView::new().push(Element::centered_text(
            "Round 2 of 3",
            ViewRect::new(40.0, 30.0, 1200.0, 50.0),
            32.0,
            Tone::Muted,
        ));

        let mut f = frame();
        paint_gameplay_view(&view, &Palette::default(), &no_art, &mut f);

        assert_eq!(f.texts.len(), 1);
        let item = &f.texts[0];
        assert_eq!(item.text, "Round 2 of 3");
        assert_eq!(item.align, TextAlign::Center);
        assert_eq!(item.color, Palette::default().muted);
        assert!((item.max_width - 1200.0).abs() < 0.01);
    }

    #[test]
    fn an_image_with_no_art_draws_nothing() {
        let view = GameplayView::new().push(Element::image(
            "board",
            ViewRect::new(0.0, 0.0, 200.0, 200.0),
        ));

        let mut f = frame();
        paint_gameplay_view(&view, &Palette::default(), &no_art, &mut f);

        assert_eq!(f.primitive_count(), 0);
    }

    #[test]
    fn an_image_that_resolves_is_textured_and_tinted_white() {
        let view = GameplayView::new().push(Element::image(
            "board",
            ViewRect::new(10.0, 20.0, 300.0, 200.0),
        ));

        let mut f = frame();
        let bank = |name: &str| (name == "board").then_some(TextureId(7));
        paint_gameplay_view(&view, &Palette::default(), &bank, &mut f);

        assert_eq!(f.quads.len(), 1);
        assert_eq!(f.quads[0].texture, Some(TextureId(7)));
        assert_eq!(f.quads[0].color, Rgba::rgb(255, 255, 255));
        assert_eq!(f.quads[0].rect.width, 300.0);
    }

    #[test]
    fn elements_are_drawn_in_list_order_so_a_later_panel_covers_an_earlier_one() {
        let view = GameplayView::new()
            .push(Element::panel(ViewRect::full(), Tone::Panel))
            .push(Element::panel(
                ViewRect::new(0.0, 0.0, 10.0, 10.0),
                Tone::Accent,
            ));

        let mut f = frame();
        paint_gameplay_view(&view, &Palette::default(), &no_art, &mut f);

        assert_eq!(f.quads.len(), 2);
        assert_eq!(f.quads[1].color, Palette::default().accent);
    }

    #[test]
    fn a_palette_maps_every_tone_to_something_visible() {
        let palette = Palette::default();
        for tone in [
            Tone::Panel,
            Tone::Raised,
            Tone::Text,
            Tone::Muted,
            Tone::Accent,
            Tone::Danger,
            Tone::Success,
            Tone::Edge,
        ] {
            assert!(palette.colour(tone).a > 0, "{tone:?} must not be invisible");
        }
    }
}
