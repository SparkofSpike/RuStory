//! The RON shape a WebAssembly guest has to write for a `GameplayView`.
//!
//! This is an interface, not an implementation detail: a guest in another language cannot derive
//! it from `serde`, so it has to be written by hand, and a hand-written shape that drifted would
//! fail as an empty screen rather than as an error. The fixture here is the shape the example
//! guest emits, kept in the test suite so a change to the `serde` representation is caught.

use rustory_vn_core::prelude::*;

/// Exactly what `examples/wasm-guest` writes. Enum variants take no colon: `Panel(`, not
/// `Panel:(`.
const FROM_A_GUEST: &str = "(elements:[\
Panel(rect:(x:0.0,y:0.0,width:1280.0,height:720.0),tone:Panel,border:None),\
Text(text:\"Two doors. One opens.\",rect:(x:140.0,y:80.0,width:1000.0,height:60.0),size:40.0,tone:Text,align:Center),\
Panel(rect:(x:240.0,y:260.0,width:360.0,height:220.0),tone:Raised,border:Some(Edge)),\
Bar(rect:(x:240.0,y:560.0,width:800.0,height:14.0),fraction:0.35,tone:Accent)\
])";
#[test]
fn a_hand_written_guest_view_parses() {
    let view: GameplayView = ron::from_str(FROM_A_GUEST).expect("the documented shape must parse");

    assert_eq!(view.elements.len(), 4);
    assert!(matches!(view.elements[0], Element::Panel { .. }));
    assert!(matches!(view.elements[1], Element::Text { .. }));
    assert!(matches!(view.elements[3], Element::Bar { .. }));
}

#[test]
fn every_field_a_guest_writes_survives_the_round_trip() {
    let view: GameplayView = ron::from_str(FROM_A_GUEST).expect("parses");

    let Element::Text {
        text,
        size,
        tone,
        align,
        ..
    } = &view.elements[1]
    else {
        panic!("second element is the text run");
    };
    assert_eq!(text, "Two doors. One opens.");
    assert!((size - 40.0).abs() < f32::EPSILON);
    assert_eq!(*tone, Tone::Text);
    assert_eq!(*align, Align::Center);

    let Element::Bar { fraction, .. } = &view.elements[3] else {
        panic!("fourth element is the bar");
    };
    assert!((fraction - 0.35).abs() < 1e-6);

    let Element::Panel { border, .. } = &view.elements[2] else {
        panic!("third element is the outlined panel");
    };
    assert_eq!(*border, Some(Tone::Edge));
}

#[test]
fn what_serde_writes_is_what_a_guest_can_write() {
    // The point is equivalence, not identity in the abstract: a guest writes by hand what serde
    // would emit for the same value. If these two ever diverge, the documented ABI is wrong.
    let built = GameplayView::new()
        .push(Element::panel(ViewRect::full(), Tone::Panel))
        .push(Element::centered_text(
            "Two doors. One opens.",
            ViewRect::new(140.0, 80.0, 1000.0, 60.0),
            40.0,
            Tone::Text,
        ))
        .push(Element::outlined(
            ViewRect::new(240.0, 260.0, 360.0, 220.0),
            Tone::Raised,
            Tone::Edge,
        ))
        .push(Element::bar(
            ViewRect::new(240.0, 560.0, 800.0, 14.0),
            0.35,
            Tone::Accent,
        ));

    let from_serde = ron::ser::to_string(&built).expect("serialises");
    let hand_written: GameplayView = ron::from_str(FROM_A_GUEST).expect("parses");

    assert_eq!(from_serde, FROM_A_GUEST, "the ABI document is now wrong");
    assert_eq!(hand_written, built);
}

#[test]
fn a_variant_written_with_a_colon_is_rejected_loudly() {
    // This is the mistake that produces an empty screen instead of an error at the call site, so
    // pinning the wrong shape here is worth a test of its own.
    let wrong =
        "(elements:[(Panel:(rect:(x:0.0,y:0.0,width:1.0,height:1.0),tone:Panel,border:None))])";
    assert!(
        ron::from_str::<GameplayView>(wrong).is_err(),
        "`Panel:(` is not valid RON for a variant"
    );
}
