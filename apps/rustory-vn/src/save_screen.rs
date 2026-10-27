//! The default save screen: a grid of slots a player picks from.
//!
//! ADR 0018 records the reasoning; this is the screen itself.
//!
//! # Why a trait and not just a function
//!
//! The objective behind this layer was "provide a default, replaceable wholesale". A project whose
//! game has its own menu art does not want to patch this file — it wants its own screen. So the
//! player takes a `Box<dyn SaveScreen>`, and [`GridScreen`] is the one it gets if nobody says
//! otherwise.
//!
//! # Why it emits a `GameplayView` rather than draw calls
//!
//! The same reason a gameplay system does (ADR 0013): the screen is plain data, so layout and hit
//! testing are testable without a GPU, and both front-ends paint it with the code they already
//! have. A save screen that reached for a renderer would be the second drawing path in the
//! project.
//!
//! # Thumbnails
//!
//! The model carries pixels (see `rustory_vn_core::Thumbnail`); this screen names them
//! `save-thumb-<stem>` and emits an `Image` element. Whether that name resolves is the app's
//! business — if it has not uploaded a texture under it, the painter skips the image, which is
//! already the documented behaviour for missing art. That keeps this file free of GPU knowledge
//! and keeps a project able to supply its own pictures.

use rustory_vn_core::prelude::*;

/// What the screen is being used for.
///
/// The two modes differ in more than a word: in `Load` an empty slot does not respond, and in
/// `Save` an occupied one does — possibly after asking, which is the caller's job because only it
/// knows whether the slot holds something precious.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMode {
    Save,
    Load,
}

impl ScreenMode {
    fn title(self) -> &'static str {
        match self {
            Self::Save => "Save",
            Self::Load => "Load",
        }
    }

    /// Whether clicking `slot` should do anything.
    ///
    /// An empty slot cannot be loaded and a damaged one cannot be either; a slot holding something
    /// can always be saved over, since the caller asks first.
    fn accepts(self, summary: &SlotSummary) -> bool {
        match self {
            Self::Save => true,
            Self::Load => !summary.is_empty() && summary.problem.is_none(),
        }
    }
}

/// One card's geometry, so layout and hit testing cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardRect {
    pub slot: SaveSlot,
    pub rect: ViewRect,
}

/// A screen that turns a slot index into a draw list, and a pointer into a slot.
///
/// Both halves are required: a screen that can draw but not be clicked is a picture, and the two
/// must agree or the player clicks a card and gets a different one.
pub trait SaveScreen {
    /// The draw list for these slots.
    fn view(&self, slots: &SlotIndex, layout: &SlotLayout, mode: ScreenMode) -> GameplayView;

    /// Which slot is under this point, if any.
    fn slot_at(&self, x: f32, y: f32, layout: &SlotLayout) -> Option<SaveSlot>;
}

/// A 3×3 grid of numbered slots with the quick slot below, which is the engine's default.
#[derive(Debug, Clone, Copy, Default)]
pub struct GridScreen;

/// Geometry shared by drawing and hit testing.
///
/// One function, called from both, because computing the rectangles twice is how a click lands on
/// the card next to the one it visibly hit.
#[must_use]
pub fn cards(layout: &SlotLayout) -> Vec<CardRect> {
    const COLUMNS: usize = 3;
    const CARD_W: f32 = 352.0;
    const CARD_H: f32 = 148.0;
    const GAP_X: f32 = 32.0;
    const GAP_Y: f32 = 20.0;
    const LEFT: f32 = 80.0;
    const TOP: f32 = 100.0;

    let mut cards = Vec::with_capacity(layout.count() + 1);
    let mut numbered = 0;

    for slot in layout.slots() {
        if slot == SaveSlot::Quick {
            continue;
        }
        let column = numbered % COLUMNS;
        let row = numbered / COLUMNS;
        cards.push(CardRect {
            slot,
            rect: ViewRect::new(
                LEFT + column as f32 * (CARD_W + GAP_X),
                TOP + row as f32 * (CARD_H + GAP_Y),
                CARD_W,
                CARD_H,
            ),
        });
        numbered += 1;
    }

    // The quick slot spans the full width below the grid, because it is the one a player uses
    // without aiming and a wide target is the difference between a convenience and a fiddly one.
    let rows = numbered.div_ceil(COLUMNS).max(1);
    let grid_bottom = TOP + rows as f32 * (CARD_H + GAP_Y);
    cards.push(CardRect {
        slot: SaveSlot::Quick,
        rect: ViewRect::new(LEFT, grid_bottom, CARD_W * 3.0 + GAP_X * 2.0, 96.0),
    });

    cards
}

impl SaveScreen for GridScreen {
    fn view(&self, slots: &SlotIndex, layout: &SlotLayout, mode: ScreenMode) -> GameplayView {
        let mut view = GameplayView::with_capacity(cards(layout).len() * 4 + 4)
            .push(Element::panel(ViewRect::full(), Tone::Panel))
            .push(Element::text(
                mode.title(),
                ViewRect::new(80.0, 32.0, 600.0, 48.0),
                40.0,
                Tone::Text,
            ))
            .push(Element::text(
                match mode {
                    ScreenMode::Save => "Click a slot to save here.",
                    ScreenMode::Load => "Click a slot to resume it.",
                },
                ViewRect::new(560.0, 44.0, 640.0, 32.0),
                22.0,
                Tone::Muted,
            ));

        for card in cards(layout) {
            let summary = slots
                .get(card.slot)
                .cloned()
                .unwrap_or_else(|| SlotSummary::empty(card.slot));

            view = view.extend(card_view(&card, &summary, mode));
        }

        view
    }

    fn slot_at(&self, x: f32, y: f32, layout: &SlotLayout) -> Option<SaveSlot> {
        cards(layout)
            .into_iter()
            .find(|card| {
                x >= card.rect.x
                    && x < card.rect.right()
                    && y >= card.rect.y
                    && y < card.rect.bottom()
            })
            .map(|card| card.slot)
    }
}

/// One card: an outline, a slot label, a thumbnail slot, and the facts.
fn card_view(card: &CardRect, summary: &SlotSummary, mode: ScreenMode) -> GameplayView {
    let mut view = GameplayView::new();

    // An unclickable card is drawn muted rather than hidden: a player looking for their save needs
    // to see that slot 4 is empty, not to see a gap where slot 4 should be.
    let usable = mode.accepts(summary);
    let border = if usable { Tone::Edge } else { Tone::Muted };
    view = view.push(Element::outlined(card.rect, Tone::Raised, border));

    view = view.push(Element::text(
        card.slot.label(),
        ViewRect::new(card.rect.right() - 64.0, card.rect.y + 10.0, 48.0, 26.0),
        20.0,
        Tone::Muted,
    ));

    // Where a picture goes: only named when the model has one, so the painter's missing-art rule
    // stays the single answer to "what if there is no image".
    let text_left = if summary.thumbnail.is_some() {
        view = view.push(Element::image(
            format!("save-thumb-{}", card.slot.stem()),
            ViewRect::new(card.rect.x + 16.0, card.rect.y + 42.0, 100.0, 56.0),
        ));
        card.rect.x + 128.0
    } else {
        card.rect.x + 16.0
    };
    let text_width = card.rect.right() - text_left - 16.0;

    let (headline, second) = describe(summary);
    view = view.push(Element::text(
        headline,
        ViewRect::new(text_left, card.rect.y + 30.0, text_width, 34.0),
        26.0,
        if summary.problem.is_some() {
            Tone::Danger
        } else if summary.is_empty() {
            Tone::Muted
        } else {
            Tone::Text
        },
    ));
    if !second.is_empty() {
        view = view.push(Element::text(
            second,
            ViewRect::new(text_left, card.rect.y + 70.0, text_width, 50.0),
            19.0,
            Tone::Muted,
        ));
    }

    view
}

/// The two lines a card shows.
///
/// Four states, each said in its own words: empty, damaged, occupied with a line of dialogue, and
/// occupied without one. Collapsing them is how a player ends up clicking a slot that does nothing
/// with no idea why.
fn describe(summary: &SlotSummary) -> (String, String) {
    if let Some(problem) = &summary.problem {
        return ("Damaged".to_owned(), problem.clone());
    }
    if summary.is_empty() {
        return ("Empty".to_owned(), String::new());
    }

    let title = summary
        .title
        .clone()
        .unwrap_or_else(|| "(untitled)".to_owned());
    let node = summary
        .node
        .as_ref()
        .map_or_else(String::new, |node| node.to_string());

    let second = match (&summary.line, summary.language.as_deref()) {
        (Some(line), Some(language)) => format!("{line}   [{language}]"),
        (Some(line), None) => line.clone(),
        (None, Some(language)) => format!("at `{node}`   [{language}]"),
        (None, None) => format!("at `{node}`"),
    };

    (title, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> SlotLayout {
        SlotLayout::new("/saves", 9)
    }

    fn occupied(slot: SaveSlot) -> SlotSummary {
        SlotSummary {
            slot,
            saved_at_ms: Some(1),
            title: Some("The Last Train".to_owned()),
            node: Some(NodeId::from("platform")),
            language: Some("ja".to_owned()),
            line: Some("You stayed.".to_owned()),
            thumbnail: None,
            problem: None,
        }
    }

    #[test]
    fn the_grid_places_nine_numbered_slots_and_one_quick_slot() {
        let cards = cards(&layout());
        assert_eq!(cards.len(), 10, "nine numbered plus quick");
        assert_eq!(cards[0].slot, SaveSlot::Numbered(0));
        assert_eq!(cards[8].slot, SaveSlot::Numbered(8));
        assert_eq!(cards[9].slot, SaveSlot::Quick);
    }

    #[test]
    fn cards_do_not_overlap() {
        // Overlapping cards would make a click ambiguous, and the loser would be whichever
        // rectangle happened to come first.
        let cards = cards(&layout());
        for (index, a) in cards.iter().enumerate() {
            for b in &cards[index + 1..] {
                let disjoint = a.rect.right() <= b.rect.x
                    || b.rect.right() <= a.rect.x
                    || a.rect.bottom() <= b.rect.y
                    || b.rect.bottom() <= a.rect.y;
                assert!(disjoint, "{:?} overlaps {:?}", a.slot, b.slot);
            }
        }
    }

    #[test]
    fn every_card_is_inside_the_design_box() {
        let cards = cards(&layout());
        for card in &cards {
            assert!(
                card.rect.x >= 0.0
                    && card.rect.y >= 0.0
                    && card.rect.right() <= VIEW_WIDTH
                    && card.rect.bottom() <= VIEW_HEIGHT,
                "{:?} is outside the box: {:?}",
                card.slot,
                card.rect
            );
        }
    }

    #[test]
    fn clicking_the_middle_of_a_card_returns_that_card() {
        // The property that matters: drawing and hit testing share `cards`, so a click lands where
        // the player aimed. This test would fail the moment someone computed one of them twice.
        let screen = GridScreen;
        let layout = layout();
        for card in cards(&layout) {
            let centre_x = card.rect.x + card.rect.width / 2.0;
            let centre_y = card.rect.y + card.rect.height / 2.0;
            assert_eq!(
                screen.slot_at(centre_x, centre_y, &layout),
                Some(card.slot),
                "centre of {:?}",
                card.slot
            );
        }
    }

    #[test]
    fn a_click_between_cards_is_on_no_card() {
        let screen = GridScreen;
        let layout = layout();
        let cards = cards(&layout);
        // The gap to the right of the first card.
        let gap_x = cards[0].rect.right() + 1.0;
        let gap_y = cards[0].rect.y + 10.0;
        assert_eq!(screen.slot_at(gap_x, gap_y, &layout), None);
    }

    #[test]
    fn a_click_outside_everything_is_on_no_card() {
        let screen = GridScreen;
        let layout = layout();
        assert_eq!(screen.slot_at(5.0, 5.0, &layout), None);
        assert_eq!(screen.slot_at(1270.0, 5.0, &layout), None);
    }

    #[test]
    fn a_narrow_layout_produces_fewer_cards_and_still_fits() {
        // A project with three slots should get one row, not three rows with two holes.
        let layout = SlotLayout::new("/saves", 3);
        let cards = cards(&layout);
        assert_eq!(cards.len(), 4);
        let quick = cards.last().expect("quick slot");
        assert_eq!(quick.slot, SaveSlot::Quick);
        assert!(
            quick.rect.bottom() <= VIEW_HEIGHT,
            "the quick slot must not fall off the screen: {:?}",
            quick.rect
        );
    }

    #[test]
    fn an_empty_slot_says_empty_rather_than_showing_nothing() {
        let view = GridScreen.view(&SlotIndex::new(), &layout(), ScreenMode::Load);
        let texts: Vec<&str> = view.texts().collect();
        assert_eq!(
            texts.iter().filter(|text| **text == "Empty").count(),
            10,
            "every slot says so: {texts:?}"
        );
    }

    #[test]
    fn an_occupied_slot_shows_its_title_line_and_language() {
        let mut slots = SlotIndex::new();
        slots.insert(occupied(SaveSlot::Numbered(0)));

        let view = GridScreen.view(&slots, &slot_layout_one(), ScreenMode::Load);
        let texts: Vec<&str> = view.texts().collect();

        assert!(texts.contains(&"The Last Train"), "got {texts:?}");
        assert!(
            texts.iter().any(|text| text.contains("You stayed.")),
            "got {texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.contains("[ja]")),
            "the language must be visible, or a resume silently changes it: {texts:?}"
        );
    }

    fn slot_layout_one() -> SlotLayout {
        SlotLayout::new("/saves", 1)
    }

    #[test]
    fn a_damaged_slot_is_labelled_damaged_and_not_offered_for_loading() {
        let mut slots = SlotIndex::new();
        slots.insert(damaged(SaveSlot::Numbered(0), "unexpected end of file"));

        let view = GridScreen.view(&slots, &slot_layout_one(), ScreenMode::Load);
        assert!(
            view.texts().any(|text| text == "Damaged"),
            "got {:?}",
            view.texts().collect::<Vec<_>>()
        );

        let summary = slots.get(SaveSlot::Numbered(0)).expect("present");
        assert!(
            !ScreenMode::Load.accepts(summary),
            "a file that cannot be read must not be clickable in Load"
        );
        assert!(
            ScreenMode::Save.accepts(summary),
            "saving over it is how a player recovers from a damaged slot"
        );
    }

    #[test]
    fn an_empty_slot_is_not_clickable_in_load_but_is_in_save() {
        let empty = SlotSummary::empty(SaveSlot::Numbered(0));
        assert!(!ScreenMode::Load.accepts(&empty));
        assert!(ScreenMode::Save.accepts(&empty));
    }

    #[test]
    fn the_two_modes_say_which_one_you_are_in() {
        let layout = slot_layout_one();
        for (mode, expected) in [(ScreenMode::Save, "Save"), (ScreenMode::Load, "Load")] {
            let view = GridScreen.view(&SlotIndex::new(), &layout, mode);
            assert!(
                view.texts().any(|text| text == expected),
                "{mode:?} must say so"
            );
        }
    }

    #[test]
    fn a_slot_with_a_thumbnail_asks_for_an_image_named_after_it() {
        // The model carries pixels; the screen names them. Whether the name resolves is the app's
        // job, which is what keeps this file free of GPU knowledge.
        let mut summary = occupied(SaveSlot::Numbered(2));
        summary.thumbnail = Thumbnail::new(2, 2, vec![0; 16]);
        let mut slots = SlotIndex::new();
        slots.insert(summary);

        // A layout that actually defines slot 3, or the card is not drawn at all and the test
        // would pass for the wrong reason.
        let layout = SlotLayout::new("/saves", 3);
        let view = GridScreen.view(&slots, &layout, ScreenMode::Load);
        let images: Vec<String> = view
            .elements
            .iter()
            .filter_map(|element| match element {
                Element::Image { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();

        assert_eq!(images, ["save-thumb-slot02"], "got {images:?}");
    }

    #[test]
    fn a_slot_without_a_thumbnail_asks_for_no_image() {
        let mut slots = SlotIndex::new();
        slots.insert(occupied(SaveSlot::Numbered(0)));
        let view = GridScreen.view(&slots, &slot_layout_one(), ScreenMode::Load);
        let images = view
            .elements
            .iter()
            .filter(|element| matches!(element, Element::Image { .. }))
            .count();
        assert_eq!(images, 0);
    }
}
