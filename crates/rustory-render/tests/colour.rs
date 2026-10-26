//! Colour correctness.
//!
//! The render target is `Bgra8UnormSrgb`, which means wgpu treats every value written
//! to it as *linear* and encodes it to sRGB on the way out. A colour specified as
//! `12/255` therefore does not arrive as `12/255`; it arrives brighter.
//!
//! These tests pin what actually lands in the framebuffer, so the renderer's colour
//! handling cannot drift silently. A visual novel's mood lives in its colour grading —
//! a dialogue box that is supposed to be near-black and renders as mid-grey is a bug
//! nobody would have written a ticket for, but everybody would notice.

use rustory_render::{Frame, Quad, Rect, Renderer, Rgba};

fn renderer_or_skip() -> Option<Renderer> {
    match Renderer::new_headless(16, 16) {
        Ok(renderer) => Some(renderer),
        Err(err) => {
            eprintln!("skipping colour test: {err}");
            None
        }
    }
}

/// Fill the frame with a solid quad of the clear colour and compare.
#[test]
fn a_textured_quad_reproduces_the_source_pixels() {
    // The full round trip: sRGB bytes → texture upload → sRGB-decoded sample → linear
    // framebuffer → sRGB-encoded readback. Every step must cancel out, or a project's art
    // silently changes colour when it is drawn.
    let Some(mut renderer) = renderer_or_skip() else {
        return;
    };

    let source = Rgba::rgb(38, 48, 74);
    let texture = rustory_render::Rgba8Image::solid(4, 4, source);
    let id = renderer.upload_texture(&texture, "probe");

    let mut frame = Frame::new(16, 16).with_clear(Rgba::rgb(0, 0, 0));
    frame.push_quad(Quad::textured(
        Rect::new(0.0, 0.0, 16.0, 16.0),
        id,
        Rgba::rgb(255, 255, 255),
    ));

    let image = renderer.render_offscreen(&frame).expect("render");
    let pixel = image.pixel(8, 8).expect("centre");

    // Exact equality is reasonable here because every conversion is the standard one
    // and the value passes through it twice. A tolerance of 1 absorbs rounding in the
    // encoding table without hiding a colour-space mistake, which is off by tens.
    assert!(
        (pixel.r as i32 - source.r as i32).abs() <= 1
            && (pixel.g as i32 - source.g as i32).abs() <= 1
            && (pixel.b as i32 - source.b as i32).abs() <= 1,
        "a texture should render as itself: asked {source:?}, got {pixel:?}"
    );
}

/// What the clear colour actually becomes, as an 8-bit channel.
fn clear_result(color: Rgba) -> Option<u8> {
    let mut renderer = renderer_or_skip()?;
    let frame = Frame::new(16, 16).with_clear(color);
    let image = renderer.render_offscreen(&frame).expect("render");
    image.pixel(8, 8).map(|pixel| pixel.r)
}

#[test]
fn a_near_black_clear_colour_stays_near_black() {
    let Some(value) = clear_result(Rgba::rgb(12, 14, 20)) else {
        return;
    };

    assert!(
        value <= 40,
        "a clear colour of 12 should look near-black, but landed at {value}. \
         If this is ~60 the value was treated as linear and re-encoded to sRGB."
    );
}

#[test]
fn pure_black_is_still_pure_black() {
    // The one value where the conversion is a no-op, so a failure here means something
    // other than colour space is wrong.
    let Some(value) = clear_result(Rgba::rgb(0, 0, 0)) else {
        return;
    };
    assert_eq!(value, 0);
}

#[test]
fn pure_white_is_still_pure_white() {
    let Some(value) = clear_result(Rgba::rgb(255, 255, 255)) else {
        return;
    };
    assert_eq!(value, 255);
}

#[test]
fn a_solid_quad_matches_its_clear_colour_of_the_same_value() {
    // The clearest signal that quads and clears disagree: fill the frame with a quad
    // of colour X over a clear of colour X. If the two paths treat colour differently,
    // the result is not X.
    let Some(mut renderer) = renderer_or_skip() else {
        return;
    };

    let colour = Rgba::rgb(64, 80, 96);
    let mut frame = Frame::new(16, 16).with_clear(colour);
    frame.push_quad(Quad::solid(Rect::new(0.0, 0.0, 16.0, 16.0), colour));

    let image = renderer.render_offscreen(&frame).expect("render");
    let cleared = image.pixel(1, 1).expect("cleared corner");
    let covered = image.pixel(8, 8).expect("quad centre");

    assert_eq!(
        covered.r, cleared.r,
        "a solid quad of the clear colour must not change the pixel: \
         quad={covered:?} clear={cleared:?}"
    );
    assert_eq!(covered.g, cleared.g);
    assert_eq!(covered.b, cleared.b);
}
