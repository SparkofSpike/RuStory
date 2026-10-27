//! Reading and writing save slots.
//!
//! The model is `rustory_vn_core::slots`; this is the part that touches the filesystem. Kept
//! apart for the reason the core has no `std::fs`: the model is testable in memory, and the bugs
//! that live here are the ones a filesystem has — a missing directory, a half-written file, a
//! save from three versions ago.
//!
//! # Writing is atomic
//!
//! A save is written to a temporary file beside the target and renamed over it. A process killed
//! mid-write therefore leaves either the old save or the new one, never a truncated file that
//! loads as garbage. Saves are small enough that the copy costs nothing, and "my save is corrupt"
//! is the one failure a player cannot work around.

use std::collections::BTreeMap;

use rustory_vn_core::{
    SaveSlot, SaveState, SlotIndex, SlotLayout, SlotSummary, Story, damaged, summarise,
};

/// Why a slot could not be written.
#[derive(Debug)]
pub enum SlotError {
    /// The directory could not be created or written to.
    Io {
        path: std::path::PathBuf,
        message: String,
    },
    /// The save could not be serialised, which means the format and the code disagree.
    Serialise(String),
}

impl std::fmt::Display for SlotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "cannot write {}: {message}", path.display()),
            Self::Serialise(message) => write!(f, "cannot serialise the save: {message}"),
        }
    }
}

impl std::error::Error for SlotError {}

/// The extra facts a slot screen shows that a save does not carry.
///
/// Passed in rather than stored in `SaveState` because they are presentation: the line of dialogue
/// a menu shows is whatever the caller was looking at, and a thumbnail is pixels the core never
/// makes.
#[derive(Debug, Clone, Default)]
pub struct SlotContext {
    /// The story's title at the time of saving.
    pub title: Option<String>,
    /// A short line of dialogue, for the row's second line.
    pub line: Option<String>,
}

/// Save `state` into `slot`, creating the directory if it is missing.
///
/// `saved_at_ms` is the caller's clock: the core has no notion of time, and a save's timestamp is
/// a fact about this machine rather than about the story.
pub fn write(
    layout: &SlotLayout,
    slot: SaveSlot,
    state: &SaveState,
    context: &SlotContext,
    saved_at_ms: u64,
) -> Result<SlotSummary, SlotError> {
    std::fs::create_dir_all(layout.dir()).map_err(|err| SlotError::Io {
        path: layout.dir().to_path_buf(),
        message: err.to_string(),
    })?;

    let text = state
        .to_ron()
        .map_err(|err| SlotError::Serialise(err.to_string()))?;

    let target = layout.path(slot);
    // The temporary file lives in the same directory as the target, because a rename is only
    // atomic within one filesystem.
    let temporary = target.with_extension("ron.tmp");
    std::fs::write(&temporary, &text).map_err(|err| SlotError::Io {
        path: temporary.clone(),
        message: err.to_string(),
    })?;
    std::fs::rename(&temporary, &target).map_err(|err| SlotError::Io {
        path: target.clone(),
        message: err.to_string(),
    })?;

    Ok(summarise(
        slot,
        state,
        context.title.clone(),
        saved_at_ms,
        context.line.clone(),
    ))
}

/// Read one slot.
///
/// Returns `Ok(None)` for a slot with no file, and an error for a file that exists but cannot be
/// parsed. The caller turns that error into a damaged slot rather than a failure, because one bad
/// file must not stop a player loading a different one.
pub fn read(layout: &SlotLayout, slot: SaveSlot) -> std::io::Result<Option<SaveState>> {
    let path = layout.path(slot);
    match std::fs::read_to_string(&path) {
        Ok(text) => match SaveState::from_ron(&text) {
            Ok(save) => Ok(Some(save)),
            Err(err) => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Every slot's summary, for a save screen.
///
/// A slot that cannot be read becomes a damaged entry rather than an error: one bad file must not
/// stop a player from loading a different one. Timestamps are left at zero here; see
/// [`read_all_with_times`] for the version a screen uses.
#[must_use]
pub fn read_all(layout: &SlotLayout, story: &Story) -> SlotIndex {
    let title = story.meta.title.clone();
    let checksum = story.checksum();
    let mut index = SlotIndex::new();

    for slot in layout.slots() {
        match read(layout, slot) {
            Ok(Some(save)) => {
                // A save from a different version of the story: the checksum decides whether
                // resuming it is possible, and a slot screen should say so before the player
                // picks it.
                let problem = (save.story_checksum != checksum)
                    .then(|| "written for a different version of this story".to_owned());
                let summary = SlotSummary {
                    problem,
                    ..summarise(slot, &save, title.clone(), 0, None)
                };
                index.insert(summary);
            }
            Ok(None) => index.insert(SlotSummary::empty(slot)),
            Err(err) => index.insert(damaged(slot, err.to_string())),
        }
    }

    index
}

/// The same as [`read_all`], with each slot's modification time.
///
/// Separate because reading metadata is a second filesystem call per slot, and a caller that only
/// wants the list should not pay for it.
///
/// The save format has no timestamp — it never needed one. The file's modification time answers the
/// same question without adding a field to a format to serve one screen.
#[must_use]
pub fn read_all_with_times(layout: &SlotLayout, story: &Story) -> SlotIndex {
    let mut index = read_all(layout, story);

    let mut times: BTreeMap<SaveSlot, u64> = BTreeMap::new();
    for slot in layout.slots() {
        if let Ok(metadata) = std::fs::metadata(layout.path(slot))
            && let Ok(modified) = metadata.modified()
            && let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            times.insert(slot, since.as_millis() as u64);
        }
    }

    for slot in layout.slots() {
        if let Some(at) = times.get(&slot)
            && let Some(summary) = index.get(slot).cloned()
            && !summary.is_empty()
        {
            index.insert(SlotSummary {
                saved_at_ms: Some(*at),
                ..summary
            });
        }
    }

    index
}

/// Delete a slot's file.
///
/// Missing is success: the caller asked for the slot to be empty, and it is.
pub fn delete(layout: &SlotLayout, slot: SaveSlot) -> std::io::Result<()> {
    match std::fs::remove_file(layout.path(slot)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustory_vn_core::{Beat, Exit, Node, NodeId, SAVE_VERSION};

    fn story() -> Story {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            NodeId::from("start"),
            Node {
                beats: vec![Beat::Say {
                    speaker: None,
                    text: "hi".into(),
                }],
                exit: Exit::End { ending: None },
            },
        );
        Story::new("start", nodes)
    }

    fn state() -> SaveState {
        SaveState {
            version: SAVE_VERSION,
            story_checksum: 7,
            cursor: NodeId::from("start"),
            beat_index: 0,
            mode: rustory_vn_core::Mode::AwaitingAdvance,
            revealed: 1,
            stage: rustory_vn_core::Stage::default(),
            music: None,
            visited: vec![NodeId::from("start")],
            choices: Vec::new(),
            story: None,
            language: None,
            stats: rustory_vn_core::Stats::default(),
            gameplay: None,
        }
    }

    /// A scratch directory that cleans itself up.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("rustory-slots-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_written_slot_reads_back_unchanged() {
        let scratch = Scratch::new("roundtrip");
        let layout = SlotLayout::new(&scratch.0, 3);
        let original = state();

        let summary = write(
            &layout,
            SaveSlot::Numbered(1),
            &original,
            &SlotContext {
                title: Some("Story".to_owned()),
                line: Some("hi".to_owned()),
            },
            1_000,
        )
        .expect("writes");

        assert_eq!(summary.slot, SaveSlot::Numbered(1));
        assert_eq!(summary.saved_at_ms, Some(1_000));
        assert_eq!(summary.node, Some(NodeId::from("start")));

        let loaded = read(&layout, SaveSlot::Numbered(1))
            .expect("reads")
            .expect("present");
        assert_eq!(loaded, original);
    }

    #[test]
    fn writing_creates_the_directory_on_a_first_save() {
        let scratch = Scratch::new("mkdir");
        let layout = SlotLayout::new(scratch.0.join("nested/saves"), 2);
        assert!(!layout.dir().exists(), "the test must start without it");

        write(
            &layout,
            SaveSlot::Quick,
            &state(),
            &SlotContext::default(),
            0,
        )
        .expect("writes");
        assert!(layout.dir().is_dir());
    }

    #[test]
    fn an_empty_slot_reads_as_nothing_rather_than_an_error() {
        let scratch = Scratch::new("empty");
        let layout = SlotLayout::new(&scratch.0, 2);
        assert!(
            read(&layout, SaveSlot::Numbered(0))
                .expect("no error")
                .is_none()
        );
    }

    #[test]
    fn a_corrupt_file_becomes_a_damaged_slot_not_a_failure() {
        // One bad file must not stop a player loading a different one.
        let scratch = Scratch::new("corrupt");
        let layout = SlotLayout::new(&scratch.0, 2);
        std::fs::create_dir_all(layout.dir()).expect("mkdir");
        std::fs::write(layout.path(SaveSlot::Numbered(0)), "not a save").expect("write");

        let index = read_all(&layout, &story());
        let summary = index.get(SaveSlot::Numbered(0)).expect("present");
        assert!(summary.problem.is_some(), "got {summary:?}");
        assert!(!summary.is_empty(), "damaged is not empty");
    }

    #[test]
    fn a_save_written_for_another_story_is_flagged_before_it_is_picked() {
        let scratch = Scratch::new("checksum");
        let layout = SlotLayout::new(&scratch.0, 2);
        let mut other = state();
        other.story_checksum = 999;
        write(
            &layout,
            SaveSlot::Numbered(0),
            &other,
            &SlotContext::default(),
            0,
        )
        .expect("writes");

        let index = read_all(&layout, &story());
        let summary = index.get(SaveSlot::Numbered(0)).expect("present");
        assert!(
            summary
                .problem
                .as_deref()
                .unwrap_or("")
                .contains("different version"),
            "got {summary:?}"
        );
    }

    #[test]
    fn a_slot_written_twice_holds_only_the_second_save() {
        // Overwriting is the normal case, and the temporary file must not survive it.
        let scratch = Scratch::new("overwrite");
        let layout = SlotLayout::new(&scratch.0, 1);

        let mut first = state();
        first.beat_index = 1;
        write(
            &layout,
            SaveSlot::Numbered(0),
            &first,
            &SlotContext::default(),
            1,
        )
        .expect("first");

        let mut second = state();
        second.beat_index = 2;
        write(
            &layout,
            SaveSlot::Numbered(0),
            &second,
            &SlotContext::default(),
            2,
        )
        .expect("second");

        let loaded = read(&layout, SaveSlot::Numbered(0))
            .expect("reads")
            .expect("present");
        assert_eq!(loaded.beat_index, 2);
        assert!(
            !layout
                .path(SaveSlot::Numbered(0))
                .with_extension("ron.tmp")
                .exists(),
            "the temporary file must be gone"
        );
    }

    #[test]
    fn deleting_an_empty_slot_is_success() {
        // The caller asked for the slot to be empty, and it is.
        let scratch = Scratch::new("delete");
        let layout = SlotLayout::new(&scratch.0, 1);
        delete(&layout, SaveSlot::Numbered(0)).expect("not found is fine");
    }

    #[test]
    fn deleting_removes_the_file() {
        let scratch = Scratch::new("delete2");
        let layout = SlotLayout::new(&scratch.0, 1);
        write(
            &layout,
            SaveSlot::Numbered(0),
            &state(),
            &SlotContext::default(),
            0,
        )
        .expect("write");
        assert!(layout.path(SaveSlot::Numbered(0)).exists());

        delete(&layout, SaveSlot::Numbered(0)).expect("deletes");
        assert!(!layout.path(SaveSlot::Numbered(0)).exists());
    }

    #[test]
    fn modification_times_fill_the_newest_slot() {
        let scratch = Scratch::new("times");
        let layout = SlotLayout::new(&scratch.0, 2);
        // The save's checksum must match the story, or both slots are flagged as written for a
        // different version and `newest()` correctly refuses them — which is what this test
        // caught the first time it ran.
        let story = story();
        let matching = SaveState {
            story_checksum: story.checksum(),
            ..state()
        };

        write(
            &layout,
            SaveSlot::Numbered(0),
            &matching,
            &SlotContext::default(),
            0,
        )
        .expect("first");
        // A distinct mtime, so the ordering is not a coin flip on a fast filesystem.
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(
            &layout,
            SaveSlot::Numbered(1),
            &matching,
            &SlotContext::default(),
            0,
        )
        .expect("second");

        let index = read_all_with_times(&layout, &story);
        assert_eq!(
            index.newest().map(|summary| summary.slot),
            Some(SaveSlot::Numbered(1)),
            "the later write must be the newest, got {:?}",
            index.get(SaveSlot::Numbered(0)).map(|s| &s.problem)
        );
    }
}
