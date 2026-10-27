//! Save slots: the list a player picks from, and the metadata a screen needs.
//!
//! ADR 0006 settled what a save *contains*. This settles what a set of them is.
//!
//! # Why this is in the core and the files are not
//!
//! A slot is a position plus the few facts a menu shows to help someone choose: which story,
//! where it was, when, and a thumbnail. Deciding what those facts are is a game-model decision;
//! writing them to disk is not. So [`SaveSlot`] and [`SlotSummary`] live here, with no `std::fs`
//! anywhere, and the caller does the I/O. That keeps the core testable in memory, which is the
//! rule every other module already follows.
//!
//! # Two layers, and why not one
//!
//! - [`SlotLayout`] answers "which file is slot 3", for a project that wants numbered slots.
//! - [`SlotIndex`] answers "what is in the slots I have", for the screen that lists them.
//!
//! A single type would have to be both a naming scheme and a cache, and the first bug would be a
//! screen showing a slot that no longer exists on disk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::ids::NodeId;
use crate::save::SaveState;

/// How many numbered slots a project gets by default.
///
/// Nine: one keypress each (`1`–`9`), and enough for "before the boss", "the ending I liked" and
/// the six in between. A project that wants more changes the number; nothing else depends on it.
pub const DEFAULT_SLOTS: usize = 9;

/// A numbered save slot, or the quick slot the window writes with F5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SaveSlot {
    /// `1`–`9` in the interface, `0`–`8` in this type.
    Numbered(usize),
    /// The slot F5 writes and F9 reads: one per story, overwritten without asking.
    Quick,
}

impl SaveSlot {
    /// The label a menu shows.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Numbered(index) => (index + 1).to_string(),
            Self::Quick => "quick".to_owned(),
        }
    }

    /// The file stem for this slot, without an extension.
    ///
    /// Numbered slots are zero-padded so a directory listing sorts the way the menu does; a
    /// listing that reads `10` between `1` and `2` is a small thing that makes a folder look
    /// untended.
    #[must_use]
    pub fn stem(&self) -> String {
        match self {
            Self::Numbered(index) => format!("slot{index:02}"),
            Self::Quick => "quick".to_owned(),
        }
    }

    /// Parse a stem back into a slot, the inverse of [`SaveSlot::stem`].
    #[must_use]
    pub fn from_stem(stem: &str) -> Option<Self> {
        if stem == "quick" {
            return Some(Self::Quick);
        }
        let digits = stem.strip_prefix("slot")?;
        digits.parse().ok().map(Self::Numbered)
    }
}

/// Where a project's saves live, and what they are called.
///
/// Immutable and cheap to copy: a directory, an extension, and how many numbered slots exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotLayout {
    dir: PathBuf,
    extension: String,
    slots: usize,
}

impl SlotLayout {
    /// `count` numbered slots plus a quick slot, under `dir`, as `.ron` files.
    ///
    /// `.ron` rather than a binary format for the same reason story files are RON: a save a person
    /// can open is a save they can diagnose, and a save is small enough that the bytes do not
    /// matter.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>, count: usize) -> Self {
        Self {
            dir: dir.into(),
            extension: "ron".to_owned(),
            slots: count,
        }
    }

    /// The project's own layout: `<story dir>/saves/`, nine numbered slots.
    ///
    /// Beside the story rather than in a global directory, so a project is one folder to copy,
    /// back up or delete — and so two projects cannot overwrite each other's slot 3.
    #[must_use]
    pub fn beside(story: &Path) -> Self {
        let dir = story.parent().unwrap_or(Path::new(".")).join("saves");
        Self::new(dir, DEFAULT_SLOTS)
    }

    /// How many numbered slots exist.
    #[must_use]
    pub fn count(&self) -> usize {
        self.slots
    }

    /// The directory saves are written to.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The path of one slot.
    #[must_use]
    pub fn path(&self, slot: SaveSlot) -> PathBuf {
        self.dir.join(format!("{}.{}", slot.stem(), self.extension))
    }

    /// Every slot this layout defines, numbered first then the quick slot.
    pub fn slots(&self) -> impl Iterator<Item = SaveSlot> {
        (0..self.slots)
            .map(SaveSlot::Numbered)
            .chain(std::iter::once(SaveSlot::Quick))
    }

    /// Whether a slot is one this layout defines.
    ///
    /// A save file whose number is past the count was written by a project with more slots. It
    /// still loads if asked for by name; it is just not offered in the menu, which would otherwise
    /// silently grow a tenth entry nobody configured.
    #[must_use]
    pub fn contains(&self, slot: SaveSlot) -> bool {
        match slot {
            SaveSlot::Numbered(index) => index < self.slots,
            SaveSlot::Quick => true,
        }
    }
}

/// What a save-slot screen shows for one slot.
///
/// Every field is something a player uses to decide: which story, where they were, when, and a
/// picture. An empty slot has none of them, and [`SlotSummary::is_empty`] says so rather than
/// making the screen infer it from a `None` title.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotSummary {
    pub slot: SaveSlot,
    /// `None` for an empty slot.
    pub saved_at_ms: Option<u64>,
    /// The story title at the time of saving, if the story had one.
    pub title: Option<String>,
    /// The node the player was in. A chapter name in practice, since authors name nodes.
    pub node: Option<NodeId>,
    /// Which language it was played in, so a screen can mark a save that will change language.
    pub language: Option<String>,
    /// A short line of dialogue, for the row's second line.
    pub line: Option<String>,
    /// A thumbnail, if the caller rendered one. The core never makes pixels; it carries them.
    pub thumbnail: Option<Thumbnail>,
    /// Why this slot could not be read, if it could not.
    ///
    /// A corrupt save is a slot a player can see and delete, not a crash and not an invisible
    /// hole. Keeping the reason means the screen can say "damaged" instead of "empty", which are
    /// very different things to a player who remembers saving there.
    pub problem: Option<String>,
}

impl SlotSummary {
    /// A slot with nothing in it.
    #[must_use]
    pub fn empty(slot: SaveSlot) -> Self {
        Self {
            slot,
            saved_at_ms: None,
            title: None,
            node: None,
            language: None,
            line: None,
            thumbnail: None,
            problem: None,
        }
    }

    /// Whether this slot holds no save at all.
    ///
    /// A damaged slot is **not** empty: it has something in it that could not be read, and the
    /// screen must not offer to overwrite it without saying so.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.saved_at_ms.is_none() && self.problem.is_none()
    }

    /// A one-line description for a log or a CLI listing.
    #[must_use]
    pub fn describe(&self) -> String {
        if let Some(problem) = &self.problem {
            return format!("{}: damaged — {problem}", self.slot.label());
        }
        if self.is_empty() {
            return format!("{}: empty", self.slot.label());
        }
        format!(
            "{}: {} at `{}`{}",
            self.slot.label(),
            self.title.as_deref().unwrap_or("(untitled)"),
            self.node.as_ref().map_or("?", NodeId::as_str),
            self.line
                .as_ref()
                .map(|line| format!(" — {line}"))
                .unwrap_or_default()
        )
    }
}

/// A save screen's thumbnail.
///
/// Straight RGBA at the size the renderer produced. The core does not scale it: choosing a size is
/// the screen's decision, and a core that resampled images would need an image library for a
/// feature it does not own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, RGBA.
    pub pixels: Vec<u8>,
}

impl Thumbnail {
    /// A thumbnail from raw pixels, or `None` if the buffer does not match the size.
    ///
    /// Checked at construction because a mismatched buffer would be read out of bounds by
    /// whoever draws it, and that is a crash in a screen rather than a message.
    #[must_use]
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        (pixels.len() == expected).then_some(Self {
            width,
            height,
            pixels,
        })
    }
}

/// Every slot's summary, keyed by slot.
///
/// A map rather than a vector because the caller's slots are identified, not ordered: a screen
/// that wants order calls [`SlotLayout::slots`] and looks each one up, which is the only ordering
/// that cannot disagree with the layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlotIndex {
    entries: BTreeMap<SaveSlot, SlotSummary>,
}

impl SlotIndex {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one slot's state.
    pub fn insert(&mut self, summary: SlotSummary) {
        self.entries.insert(summary.slot, summary);
    }

    #[must_use]
    pub fn get(&self, slot: SaveSlot) -> Option<&SlotSummary> {
        self.entries.get(&slot)
    }

    /// How many slots hold something, damaged ones included.
    ///
    /// Damaged counts because the player has to be able to find and delete it; a count that
    /// excluded it would leave a file on disk that no screen admits to.
    #[must_use]
    pub fn occupied(&self) -> usize {
        self.entries
            .values()
            .filter(|summary| !summary.is_empty())
            .count()
    }

    /// The most recent save, which is what a "continue" button needs.
    #[must_use]
    pub fn newest(&self) -> Option<&SlotSummary> {
        self.entries
            .values()
            .filter(|summary| summary.problem.is_none())
            .filter_map(|summary| summary.saved_at_ms.map(|at| (at, summary)))
            .max_by_key(|(at, _)| *at)
            .map(|(_, summary)| summary)
    }

    /// One line per slot, in layout order, for a CLI listing.
    #[must_use]
    pub fn describe_all(&self, layout: &SlotLayout) -> Vec<String> {
        layout
            .slots()
            .map(|slot| {
                self.entries
                    .get(&slot)
                    .cloned()
                    .unwrap_or_else(|| SlotSummary::empty(slot))
                    .describe()
            })
            .collect()
    }
}

/// The facts about a save that a slot screen needs, read out of a [`SaveState`].
///
/// A save already carries the node, the language and the stats; this pulls the parts a menu shows
/// so the screen does not reach into the save format itself. When the format gains a field, this
/// is the one place that decides whether the menu cares.
#[must_use]
pub fn summarise(
    slot: SaveSlot,
    save: &SaveState,
    title: Option<String>,
    saved_at_ms: u64,
    line: Option<String>,
) -> SlotSummary {
    SlotSummary {
        slot,
        saved_at_ms: Some(saved_at_ms),
        title,
        node: Some(save.cursor.clone()),
        language: save.language.clone(),
        line,
        thumbnail: None,
        problem: None,
    }
}

/// A save that could not be read, as a slot summary.
#[must_use]
pub fn damaged(slot: SaveSlot, reason: impl Into<String>) -> SlotSummary {
    SlotSummary {
        problem: Some(reason.into()),
        ..SlotSummary::empty(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::Stats;
    use crate::narrative::Mode;
    use crate::save::SAVE_VERSION;
    use crate::stage::Stage;

    fn save_at(node: &str) -> SaveState {
        SaveState {
            version: SAVE_VERSION,
            story_checksum: 1,
            story: None,
            cursor: NodeId::from(node),
            beat_index: 0,
            mode: Mode::AwaitingAdvance,
            revealed: 0,
            stage: Stage::default(),
            music: None,
            visited: Vec::new(),
            choices: Vec::new(),
            language: None,
            stats: Stats::default(),
            gameplay: None,
        }
    }

    #[test]
    fn numbered_slots_are_zero_padded_so_a_listing_sorts_like_the_menu() {
        assert_eq!(SaveSlot::Numbered(0).stem(), "slot00");
        assert_eq!(SaveSlot::Numbered(9).stem(), "slot09");
        assert_eq!(SaveSlot::Numbered(10).stem(), "slot10");
    }

    #[test]
    fn a_stem_round_trips_back_to_its_slot() {
        for slot in [
            SaveSlot::Numbered(0),
            SaveSlot::Numbered(42),
            SaveSlot::Quick,
        ] {
            assert_eq!(SaveSlot::from_stem(&slot.stem()), Some(slot));
        }
        assert_eq!(SaveSlot::from_stem("save"), None);
        assert_eq!(SaveSlot::from_stem("slotxx"), None);
    }

    #[test]
    fn a_label_is_what_a_player_presses_not_an_internal_index() {
        assert_eq!(SaveSlot::Numbered(0).label(), "1");
        assert_eq!(SaveSlot::Numbered(8).label(), "9");
        assert_eq!(SaveSlot::Quick.label(), "quick");
    }

    #[test]
    fn the_layout_defines_a_path_per_slot_under_one_directory() {
        let layout = SlotLayout::new("/saves", 3);
        assert_eq!(
            layout.path(SaveSlot::Numbered(0)),
            PathBuf::from("/saves/slot00.ron")
        );
        assert_eq!(
            layout.path(SaveSlot::Quick),
            PathBuf::from("/saves/quick.ron")
        );
        assert_eq!(
            layout.slots().collect::<Vec<_>>(),
            [
                SaveSlot::Numbered(0),
                SaveSlot::Numbered(1),
                SaveSlot::Numbered(2),
                SaveSlot::Quick
            ]
        );
    }

    #[test]
    fn a_slot_past_the_count_is_not_offered_but_the_quick_slot_always_is() {
        let layout = SlotLayout::new("/saves", 2);
        assert!(layout.contains(SaveSlot::Numbered(1)));
        assert!(!layout.contains(SaveSlot::Numbered(2)));
        assert!(layout.contains(SaveSlot::Quick));
    }

    #[test]
    fn a_layout_beside_a_story_puts_saves_next_to_it() {
        let layout = SlotLayout::beside(Path::new("/project/stories/demo.ron"));
        assert_eq!(layout.dir(), Path::new("/project/stories/saves"));
        assert_eq!(layout.count(), DEFAULT_SLOTS);
    }

    #[test]
    fn an_empty_slot_and_a_damaged_slot_are_different_things() {
        let empty = SlotSummary::empty(SaveSlot::Numbered(0));
        assert!(empty.is_empty());
        assert!(empty.problem.is_none());

        let broken = damaged(SaveSlot::Numbered(0), "unexpected end of file");
        assert!(!broken.is_empty(), "a damaged slot holds something");
        assert!(broken.describe().contains("damaged"));
        assert!(broken.describe().contains("unexpected end"));
    }

    #[test]
    fn a_summary_reads_what_a_menu_shows_out_of_a_save() {
        let mut save = save_at("chapter_two");
        save.language = Some("ja".to_owned());
        let summary = summarise(
            SaveSlot::Numbered(2),
            &save,
            Some("The Last Train".to_owned()),
            1_700_000_000_000,
            Some("You stayed.".to_owned()),
        );

        assert_eq!(summary.node, Some(NodeId::from("chapter_two")));
        assert_eq!(summary.language.as_deref(), Some("ja"));
        assert_eq!(
            summary.describe(),
            "3: The Last Train at `chapter_two` — You stayed."
        );
    }

    #[test]
    fn the_newest_slot_ignores_damaged_ones() {
        // "Continue" must resume a save that loads. Offering the most recently *written* file
        // even when it is unreadable would make the button fail on press.
        let mut index = SlotIndex::new();
        index.insert(summarise(
            SaveSlot::Numbered(0),
            &save_at("a"),
            Some("Story".to_owned()),
            100,
            None,
        ));
        index.insert(damaged(SaveSlot::Numbered(1), "garbage"));
        index.insert(summarise(
            SaveSlot::Numbered(2),
            &save_at("c"),
            Some("Story".to_owned()),
            50,
            None,
        ));

        assert_eq!(
            index.newest().map(|summary| summary.slot),
            Some(SaveSlot::Numbered(0))
        );
        assert_eq!(index.occupied(), 3, "damaged still counts as occupied");
    }

    #[test]
    fn an_empty_index_has_no_newest_slot() {
        assert!(SlotIndex::new().newest().is_none());
        assert_eq!(SlotIndex::new().occupied(), 0);
    }

    #[test]
    fn describing_all_slots_follows_the_layout_not_the_insertion_order() {
        let layout = SlotLayout::new("/saves", 2);
        let mut index = SlotIndex::new();
        // Inserted out of order, and with a slot the layout does not define.
        index.insert(summarise(
            SaveSlot::Quick,
            &save_at("q"),
            Some("Story".to_owned()),
            10,
            None,
        ));
        index.insert(summarise(
            SaveSlot::Numbered(1),
            &save_at("one"),
            Some("Story".to_owned()),
            20,
            None,
        ));

        let lines = index.describe_all(&layout);
        assert_eq!(lines.len(), 3, "two numbered plus quick");
        assert!(lines[0].starts_with("1: empty"));
        assert!(lines[1].starts_with("2: Story at `one`"));
        assert!(lines[2].starts_with("quick: Story at `q`"));
    }

    #[test]
    fn a_thumbnail_whose_buffer_does_not_match_its_size_is_refused() {
        assert!(Thumbnail::new(2, 2, vec![0; 16]).is_some());
        assert!(
            Thumbnail::new(2, 2, vec![0; 15]).is_none(),
            "a short buffer would be read out of bounds by whoever draws it"
        );
        assert!(Thumbnail::new(u32::MAX, u32::MAX, vec![0; 4]).is_none());
    }
}
