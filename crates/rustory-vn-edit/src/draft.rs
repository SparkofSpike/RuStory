//! Crash recovery: a copy of the document beside the one on disk.
//!
//! # Why this exists
//!
//! An editor that only writes on Ctrl+S loses everything typed since the last save when the
//! process dies — a power cut, a driver crash, a colleague closing the wrong window. The
//! close guard covers the polite exits and nothing else.
//!
//! So while a document has unsaved changes, the editor writes it beside the story as
//! `<story>.draft.ron`. On the next open, if a draft is there and no longer matches the story,
//! the author is offered it. Nothing is restored automatically: a file quietly coming back in
//! a form the author did not choose is its own kind of data loss.
//!
//! # Why beside the story
//!
//! Anywhere else — a temp folder, a per-user state directory — and the draft outlives the
//! project it belongs to, or gets separated from it by a move. Beside the story, deleting the
//! folder deletes the draft, and copying the project takes the recovery with it. It is added to
//! `.gitignore` for the same reason `target/` is: it is a working file, not a deliverable.
//!
//! # Why the RON form
//!
//! A draft is written with the same serialiser as a save, so a draft that cannot be parsed is
//! caught here rather than becoming a "recovered" document that will not open.

use std::path::{Path, PathBuf};

use rustory_vn_core::Story;

/// The suffix that marks a working copy. Deliberately not `.ron` alone, and deliberately
/// visible: a file a person may need to find by hand should not be hidden.
pub const SUFFIX: &str = ".draft.ron";

/// A recovery copy of one story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    path: PathBuf,
}

impl Draft {
    /// The draft that belongs to `story`, or `None` when the path has no file name.
    #[must_use]
    pub fn beside(story: &Path) -> Option<Self> {
        let name = story.file_name()?.to_str()?;
        Some(Self {
            path: story.with_file_name(format!("{name}{SUFFIX}")),
        })
    }

    /// A draft at an exact path, for tests and for tools.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether a draft is waiting.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Write the working copy.
    ///
    /// Failure is returned rather than logged, but the caller is expected to ignore it: a
    /// full disk must not stop someone editing. What it must not do is *silently* pretend the
    /// work is recoverable, which is why the caller can tell.
    pub fn write(&self, story: &Story) -> Result<(), String> {
        let text = ron::ser::to_string_pretty(story, ron::ser::PrettyConfig::default())
            .map_err(|err| err.to_string())?;
        std::fs::write(&self.path, text).map_err(|err| err.to_string())
    }

    /// Read the working copy back, or `None` when there is nothing readable there.
    ///
    /// A corrupt draft is the same as no draft: the author still has the file on disk, and
    /// offering a document that will not open would replace a real recovery with a puzzle.
    #[must_use]
    pub fn read(&self) -> Option<Story> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        rustory_vn_script::parse_story(&text, &self.path.display().to_string()).ok()
    }

    /// Remove the draft. Missing is success: the goal is that it is not there.
    pub fn discard(&self) {
        let _ = std::fs::remove_file(&self.path);
    }

    /// Put `story` back on disk and remove the draft.
    ///
    /// Returns the error rather than swallowing it, because the caller has just told the author
    /// their work was recovered and must not lie about it.
    pub fn recover_into(&self, story: &Story, target: &Path) -> Result<(), String> {
        let text = ron::ser::to_string_pretty(story, ron::ser::PrettyConfig::default())
            .map_err(|err| err.to_string())?;
        std::fs::write(target, text).map_err(|err| err.to_string())?;
        self.discard();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustory_vn_core::{Exit, Node, NodeId};

    struct Scratch {
        dir: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rustory-draft-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch");
            Self { dir }
        }

        fn story_path(&self) -> PathBuf {
            self.dir.join("chapter.ron")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn story(text: &str) -> Story {
        let mut nodes = std::collections::BTreeMap::new();
        nodes.insert(
            NodeId::from("start"),
            Node {
                beats: vec![rustory_vn_core::Beat::Say {
                    speaker: None,
                    text: text.into(),
                }],
                exit: Exit::End { ending: None },
            },
        );
        Story::new("start", nodes)
    }

    #[test]
    fn the_draft_sits_beside_the_story_with_a_visible_suffix() {
        let draft = Draft::beside(Path::new("stories/chapter-2.ron")).expect("has a file name");
        assert_eq!(
            draft.path(),
            Path::new("stories/chapter-2.ron.draft.ron"),
            "a person looking for their recovered work has to be able to find the file"
        );
    }

    #[test]
    fn a_path_without_a_file_name_has_no_draft() {
        assert!(Draft::beside(Path::new("..")).is_none());
    }

    #[test]
    fn a_written_draft_reads_back_identical() {
        let scratch = Scratch::new("round-trip");
        let draft = Draft::beside(&scratch.story_path()).expect("draft");

        assert!(!draft.exists());
        let original = story("recovered line");
        draft.write(&original).expect("write");
        assert!(draft.exists());

        assert_eq!(draft.read(), Some(original));
    }

    #[test]
    fn a_corrupt_draft_reads_as_nothing() {
        let scratch = Scratch::new("corrupt");
        let draft = Draft::beside(&scratch.story_path()).expect("draft");
        std::fs::write(draft.path(), "not a story at all").expect("write");

        assert_eq!(
            draft.read(),
            None,
            "a draft that will not open is not a recovery"
        );
    }

    #[test]
    fn discarding_removes_it_and_is_idempotent() {
        let scratch = Scratch::new("discard");
        let draft = Draft::beside(&scratch.story_path()).expect("draft");
        draft.write(&story("x")).expect("write");

        draft.discard();
        assert!(!draft.exists());
        draft.discard();
        assert!(!draft.exists());
    }

    #[test]
    fn recovering_writes_the_story_and_clears_the_draft() {
        let scratch = Scratch::new("recover");
        let draft = Draft::beside(&scratch.story_path()).expect("draft");
        let original = story("the line that was nearly lost");
        draft.write(&original).expect("write");

        draft
            .recover_into(&original, &scratch.story_path())
            .expect("recover");

        assert!(!draft.exists(), "the draft has served its purpose");
        let restored = std::fs::read_to_string(scratch.story_path()).expect("read");
        assert!(restored.contains("the line that was nearly lost"));
    }
}
