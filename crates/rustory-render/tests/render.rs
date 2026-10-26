//! Rendering tests.
//!
//! These run headlessly: a frame is rendered into an offscreen texture, read
//! back to the CPU and inspected. That is the only honest way to answer "does
//! this actually draw?" when nobody can look at a window.
//!
//! Colour is asserted relatively rather than absolutely. The target format is
//! sRGB, so a clear colour passed as 10/255 comes back re-encoded; comparing
//! exact byte values would encode a colour-space detail into the test instead of
//! the property being tested.

use rustory_render::{Frame, Quad, Rect, Renderer, Rgba, TextItem};

/// Skip rather than fail when the machine has no usable adapter.
///
/// A missing GPU is an environment fact, not a regression. CI machines that do
/// have one still exercise every assertion below.
fn renderer_or_skip(width: u32, height: u32) -> Option<Renderer> {
    match Renderer::new_headless(width, height) {
        Ok(renderer) => Some(renderer),
        Err(err) => {
            eprintln!("skipping render test: {err}");
            None
        }
    }
}

fn luminance(color: Rgba) -> u32 {
    color.r as u32 + color.g as u32 + color.b as u32
}

#[test]
fn an_empty_frame_renders_its_clear_colour() {
    let Some(mut renderer) = renderer_or_skip(64, 64) else {
        return;
    };

    let frame = Frame::new(64, 64).with_clear(Rgba::rgb(0, 0, 0));
    let image = renderer.render_offscreen(&frame).expect("render");

    assert_eq!(image.width(), 64);
    assert_eq!(image.height(), 64);
    let centre = image.pixel(32, 32).expect("pixel in range");
    assert!(luminance(centre) < 30, "cleared to black, got {centre:?}");
}

#[test]
fn a_quad_lands_where_it_was_placed() {
    let Some(mut renderer) = renderer_or_skip(64, 64) else {
        return;
    };

    let mut frame = Frame::new(64, 64).with_clear(Rgba::rgb(0, 0, 0));
    frame.push_quad(Quad::solid(
        Rect::new(8.0, 8.0, 16.0, 16.0),
        Rgba::rgb(255, 255, 255),
    ));

    let image = renderer.render_offscreen(&frame).expect("render");

    let inside = image.pixel(16, 16).expect("inside the quad");
    let outside = image.pixel(48, 48).expect("outside the quad");
    assert!(
        luminance(inside) > luminance(outside) + 100,
        "quad should be brighter inside ({inside:?}) than outside ({outside:?})"
    );

    // The boundary is where it was asked to be, not merely "somewhere bright".
    let just_inside = image.pixel(9, 9).expect("inside the corner");
    let just_outside = image.pixel(6, 6).expect("outside the corner");
    assert!(luminance(just_inside) > luminance(just_outside) + 100);
}

#[test]
fn quads_paint_in_submission_order() {
    let Some(mut renderer) = renderer_or_skip(64, 64) else {
        return;
    };

    let mut frame = Frame::new(64, 64).with_clear(Rgba::rgb(0, 0, 0));
    // Large white slab, then a smaller black one on top of it.
    frame.push_quad(Quad::solid(
        Rect::new(0.0, 0.0, 64.0, 64.0),
        Rgba::rgb(255, 255, 255),
    ));
    frame.push_quad(Quad::solid(
        Rect::new(16.0, 16.0, 32.0, 32.0),
        Rgba::rgb(0, 0, 0),
    ));

    let image = renderer.render_offscreen(&frame).expect("render");

    let covered = image.pixel(32, 32).expect("under the second quad");
    let exposed = image.pixel(4, 4).expect("outside the second quad");
    assert!(
        luminance(covered) < luminance(exposed),
        "later quads must draw on top: covered={covered:?} exposed={exposed:?}"
    );
}

#[test]
fn text_draws_pixels_inside_its_bounds() {
    let Some(mut renderer) = renderer_or_skip(320, 120) else {
        return;
    };

    let mut frame = Frame::new(320, 120).with_clear(Rgba::rgb(0, 0, 0));
    frame.push_text(
        TextItem::new("Hello, RuStory", 10.0, 10.0, 28.0).with_color(Rgba::rgb(255, 255, 255)),
    );

    let image = renderer.render_offscreen(&frame).expect("render");

    // Count bright pixels in the text band. Exact glyph shapes are cosmic-text's
    // business; that *something* was rasterized in the right region is ours.
    let mut bright = 0;
    for y in 8..60 {
        for x in 8..300 {
            if let Some(px) = image.pixel(x, y)
                && luminance(px) > 200
            {
                bright += 1;
            }
        }
    }
    assert!(
        bright > 50,
        "expected rasterized glyphs, found {bright} bright pixels"
    );

    let empty_area = image.pixel(300, 110).expect("bottom right");
    assert!(
        luminance(empty_area) < 30,
        "text must not smear across the whole frame, got {empty_area:?}"
    );
}

#[test]
fn textured_quads_sample_their_texture() {
    let Some(mut renderer) = renderer_or_skip(64, 64) else {
        return;
    };

    let texture = rustory_render::Rgba8Image::solid(4, 4, Rgba::rgb(0, 200, 0));
    let id = renderer.upload_texture(&texture, "green");

    let mut frame = Frame::new(64, 64).with_clear(Rgba::rgb(0, 0, 0));
    frame.push_quad(Quad::textured(
        Rect::new(0.0, 0.0, 64.0, 64.0),
        id,
        Rgba::rgb(255, 255, 255),
    ));

    let image = renderer.render_offscreen(&frame).expect("render");
    let px = image.pixel(32, 32).expect("centre");
    assert!(
        px.g > px.r && px.g > px.b,
        "the green texture should dominate, got {px:?}"
    );
}

#[test]
fn a_frame_can_be_saved_as_a_png() {
    let Some(mut renderer) = renderer_or_skip(80, 60) else {
        return;
    };

    let mut frame = Frame::new(80, 60).with_clear(Rgba::rgb(20, 30, 40));
    frame.push_quad(Quad::solid(
        Rect::new(10.0, 10.0, 30.0, 20.0),
        Rgba::rgb(220, 60, 60),
    ));

    let image = renderer.render_offscreen(&frame).expect("render");

    let dir = std::env::temp_dir().join("rustory-render-it");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("frame.png");
    image.save_png(&path).expect("save png");

    let reloaded = rustory_render::Rgba8Image::load_png(&path).expect("reload");
    assert_eq!(reloaded.width(), 80);
    assert_eq!(reloaded.height(), 60);

    let _ = std::fs::remove_file(&path);
}
