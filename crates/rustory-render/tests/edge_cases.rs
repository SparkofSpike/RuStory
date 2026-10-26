//! Edge cases the main tests do not cover.
//!
//! These exist because a renderer that works on tidy input and fails on odd input
//! is not finished. Each case here was chosen by asking "what would a real script
//! or a real screen size actually produce?"

use rustory_render::{Frame, Renderer, Rgba, TextItem};

fn renderer_or_skip(width: u32, height: u32) -> Option<Renderer> {
    match Renderer::new_headless(width, height) {
        Ok(renderer) => Some(renderer),
        Err(err) => {
            eprintln!("skipping render test: {err}");
            None
        }
    }
}

fn bright_pixels(image: &rustory_render::Rgba8Image) -> usize {
    let mut count = 0;
    for y in 0..image.height() {
        for x in 0..image.width() {
            if let Some(px) = image.pixel(x, y)
                && px.r as u32 + px.g as u32 + px.b as u32 > 200
            {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn an_unbounded_wrap_width_does_not_break_layout() {
    // TextItem::new leaves max_width at infinity, which is the natural default for
    // a one-line label. If cosmic-text dislikes that, every hint and every speaker
    // name would silently vanish.
    let Some(mut renderer) = renderer_or_skip(400, 120) else {
        return;
    };

    let mut frame = Frame::new(400, 120).with_clear(Rgba::rgb(0, 0, 0));
    frame.push_text(TextItem::new("unbounded", 10.0, 10.0, 30.0));

    let image = renderer.render_offscreen(&frame).expect("render");
    assert!(
        bright_pixels(&image) > 20,
        "an unbounded text item must still draw"
    );
}

#[test]
fn empty_text_draws_nothing_and_does_not_panic() {
    let Some(mut renderer) = renderer_or_skip(200, 80) else {
        return;
    };

    let mut frame = Frame::new(200, 80).with_clear(Rgba::rgb(0, 0, 0));
    frame.push_text(TextItem::new("", 10.0, 10.0, 24.0));

    let image = renderer.render_offscreen(&frame).expect("render");
    assert_eq!(
        bright_pixels(&image),
        0,
        "an empty string has no glyphs to draw"
    );
}

#[test]
fn a_long_cjk_paragraph_wraps_instead_of_overflowing() {
    let Some(mut renderer) = renderer_or_skip(400, 300) else {
        return;
    };

    // A visual novel line box is roughly this shape: narrow, and long enough to
    // need wrapping. CJK has no spaces, so wrapping depends on the shaper's line
    // breaking rather than on whitespace.
    let long = "放学后的教室只剩下两个人窗外的天色慢慢暗下来粉笔灰在光里落下".repeat(3);
    let mut frame = Frame::new(400, 300).with_clear(Rgba::rgb(0, 0, 0));
    frame.push_text(
        TextItem::new(long, 10.0, 10.0, 24.0)
            .with_color(Rgba::rgb(255, 255, 255))
            .with_wrap(360.0),
    );

    let image = renderer.render_offscreen(&frame).expect("render");

    // Text must occupy multiple rows: if wrapping failed, everything would be
    // clipped onto one line and the lower half of the box would stay black.
    let mut rows_with_text = 0;
    for y in (10..300).step_by(4) {
        let mut row_bright = 0;
        for x in 0..400 {
            if let Some(px) = image.pixel(x, y)
                && px.r as u32 + px.g as u32 + px.b as u32 > 200
            {
                row_bright += 1;
            }
        }
        if row_bright > 3 {
            rows_with_text += 1;
        }
    }
    assert!(
        rows_with_text >= 3,
        "CJK text should wrap onto several rows, found {rows_with_text}"
    );

    // And nothing may bleed past the wrap width into the right margin.
    let mut right_margin_bright = 0;
    for y in 0..300 {
        for x in 380..400 {
            if let Some(px) = image.pixel(x, y)
                && px.r as u32 + px.g as u32 + px.b as u32 > 200
            {
                right_margin_bright += 1;
            }
        }
    }
    assert_eq!(
        right_margin_bright, 0,
        "text must not overflow its wrap width"
    );
}

#[test]
fn a_degenerate_window_size_still_renders() {
    // A window can be resized to something absurd mid-frame. The renderer should
    // clamp rather than divide by zero.
    let Some(mut renderer) = renderer_or_skip(1, 1) else {
        return;
    };

    let mut frame = Frame::new(1, 1).with_clear(Rgba::rgb(10, 200, 30));
    frame.push_quad(rustory_render::Quad::solid(
        rustory_render::Rect::new(0.0, 0.0, 1.0, 1.0),
        Rgba::rgb(10, 200, 30),
    ));

    let image = renderer.render_offscreen(&frame).expect("render");
    assert_eq!(image.width(), 1);
    assert_eq!(image.height(), 1);
    let px = image.pixel(0, 0).expect("the only pixel");
    assert!(px.g > px.r, "the green clear colour should survive: {px:?}");
}

#[test]
fn a_quad_larger_than_the_target_is_clipped_not_rejected() {
    let Some(mut renderer) = renderer_or_skip(64, 64) else {
        return;
    };

    let mut frame = Frame::new(64, 64).with_clear(Rgba::rgb(0, 0, 0));
    frame.push_quad(rustory_render::Quad::solid(
        rustory_render::Rect::new(-500.0, -500.0, 2000.0, 2000.0),
        Rgba::rgb(255, 255, 255),
    ));

    let image = renderer.render_offscreen(&frame).expect("render");
    let px = image.pixel(32, 32).expect("centre");
    assert!(
        px.r > 200 && px.g > 200 && px.b > 200,
        "an oversized quad should cover the frame: {px:?}"
    );
}

#[test]
fn a_texture_id_beyond_the_uploaded_set_is_skipped() {
    // A stale id must not panic, and must not silently draw the wrong texture.
    let Some(mut renderer) = renderer_or_skip(64, 64) else {
        return;
    };

    let mut frame = Frame::new(64, 64).with_clear(Rgba::rgb(0, 0, 0));
    frame.push_quad(rustory_render::Quad::textured(
        rustory_render::Rect::new(0.0, 0.0, 64.0, 64.0),
        rustory_render::TextureId(9999),
        Rgba::rgb(255, 255, 255),
    ));

    let image = renderer.render_offscreen(&frame).expect("render");
    let px = image.pixel(32, 32).expect("centre");
    assert!(
        px.r < 40 && px.g < 40 && px.b < 40,
        "a missing texture must leave the clear colour, got {px:?}"
    );
}

#[test]
fn resizing_the_renderer_changes_the_target_size() {
    let Some(mut renderer) = renderer_or_skip(64, 64) else {
        return;
    };

    renderer.resize(320, 200);
    assert_eq!(renderer.size(), (320, 200));

    let frame = Frame::new(320, 200).with_clear(Rgba::rgb(0, 0, 0));
    let image = renderer.render_offscreen(&frame).expect("render");
    assert_eq!((image.width(), image.height()), (320, 200));
}
