//! One open story, and everything that belongs to it.
//!
//! # Why this is a separate type
//!
//! Once more than one story can be open, "which document is this state about?" becomes a
//! question the compiler can answer. A `Tab` holds every field that is *about one story*:
//! the editor, the selection, the buffers a form is typing into, the preview path, the
//! recovery draft, the filter text. Nothing on it can be about a different story.
//!
//! What is deliberately *not* here: the GPU renderer, the status line, the recent list, the
//! help window. Those are properties of the window, not of a document, and duplicating the
//! renderer per tab would mean one wgpu device per open story.
//!
//! # The bug this shape prevents
//!
//! The reference project (GEO) hit this with concurrent article streams: the abort handle and
//! the idle timer were shared variables, so when one stream finished it cleared the other's
//! timer and nulled its controller. The second stream then threw from inside a timer that no
//! longer belonged to it.
//!
//! Every mutable thing here is per tab for the same reason. `draft_due` is the interesting
//! case: with one shared deadline, editing tab A and then switching to tab B would write A's
//! draft into B's file — a data-loss bug that would be blamed on the filesystem.

use std::path::Path;
use std::time::{Duration, Instant};

use rustory_vn_core::{NodeId, Story};
use rustory_vn_edit::prelude::*;
use rustory_vn_edit::{Draft, Editor};

use crate::preview::{AssetPaths, Step};

/// How long to wait after the last edit before writing the recovery copy.
///
/// Long enough that typing a sentence does not rewrite a file per word; short enough that a
/// crash costs two seconds of work at most.
pub const DRAFT_DELAY: Duration = Duration::from_secs(2);

/// One open document.
pub struct Tab {
    /// Always present: a tab exists because a document does.
    pub editor: Editor,
    /// The node whose beats are on screen.
    pub selected: Option<NodeId>,

    // --- form buffers -----------------------------------------------------
    //
    // These hold text that is not yet a valid value — half a node id, an outcome key that is
    // not a key yet — so a field can be typed into without the document rejecting every
    // keystroke. They belong to the tab because they are about *this* story's forms.
    pub new_node_id: String,
    pub rename_buffer: String,
    pub new_outcome: String,
    pub node_filter: String,
    pub show_story_settings: bool,
    /// The gameplay config as text, and the key it was read from.
    ///
    /// `(node, story checksum)` so an undo or an outside edit refills the box instead of
    /// leaving text that no longer describes the story.
    pub config_key: Option<(NodeId, u64)>,
    pub config_buffer: String,

    // --- preview position -------------------------------------------------
    pub preview_steps: Vec<Step>,
    pub preview_status: String,
    /// Where this story's art lives, derived from its own path.
    pub assets: AssetPaths,

    // --- recovery ---------------------------------------------------------
    /// Where the working copy is written while this tab has unsaved changes.
    pub draft: Option<Draft>,
    /// When the pending write is due. Per tab — see the note at the top of this file.
    pub draft_due: Option<Instant>,
    /// Whether the author has already agreed to lose this document's comments.
    pub comment_warning_seen: bool,

    // --- this frame -------------------------------------------------------
    /// Operations produced during layout, applied before the next frame.
    ///
    /// Per tab rather than per window: an operation names a node in *this* story, and a
    /// shared queue would apply it to whichever tab happened to be active when the flush ran.
    pub pending: Vec<EditOp>,
}

impl Tab {
    /// Wrap a document that already has a home on disk, or an untitled one.
    #[must_use]
    pub fn new(editor: Editor, assets: AssetPaths) -> Self {
        let draft = if editor.document().has_file() {
            Draft::beside(editor.path())
        } else {
            // Nothing to sit a draft beside. The first save adopts one.
            None
        };

        let mut tab = Self {
            selected: None,
            editor,
            new_node_id: String::new(),
            rename_buffer: String::new(),
            new_outcome: String::new(),
            node_filter: String::new(),
            show_story_settings: false,
            config_key: None,
            config_buffer: String::new(),
            preview_steps: Vec::new(),
            preview_status: String::new(),
            assets,
            draft,
            draft_due: None,
            comment_warning_seen: false,
            pending: Vec::new(),
        };
        tab.selected = Some(tab.editor.story().start.clone());
        tab
    }

    #[must_use]
    pub fn story(&self) -> &Story {
        self.editor.story()
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.editor.path()
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.editor.is_dirty()
    }

    /// The name to show on the tab and in dialogs.
    ///
    /// "untitled" rather than the placeholder path: a story with no file yet must not look
    /// like one that has a home.
    #[must_use]
    pub fn name(&self) -> String {
        if !self.editor.document().has_file() {
            return "untitled".to_owned();
        }
        self.path().file_name().map_or_else(
            || self.path().display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    /// The node whose beats the panels should draw.
    ///
    /// Falls back to the start node when the selection has gone stale — a delete or a rename
    /// can remove the node that was selected, and a panel that renders nothing is worse than
    /// one that shows the beginning.
    #[must_use]
    pub fn selected_node(&self) -> Option<NodeId> {
        let story = self.story();
        match &self.selected {
            Some(id) if story.nodes.contains_key(id) => Some(id.clone()),
            _ => story.nodes.keys().next().cloned(),
        }
    }

    /// Select a node, resetting the preview when the selection actually changes.
    ///
    /// A step count belongs to the position it was counted at; carrying it into a different
    /// node shows a frame that has nothing to do with what was just clicked.
    ///
    /// Free rather than a method: every caller holds a borrow of `tab.editor` at the same
    /// time, and a method would take the whole tab.
    pub fn plan_selection(selected: &mut Option<NodeId>, steps: &mut Vec<Step>, id: NodeId) {
        if selected.as_ref() != Some(&id) {
            steps.clear();
        }
        *selected = Some(id);
    }

    /// Schedule a recovery write, because the document just changed.
    pub fn plan_draft(&mut self) {
        if self.draft.is_some() {
            self.draft_due = Some(Instant::now() + DRAFT_DELAY);
        }
    }

    /// Write the recovery copy if the debounce has expired.
    ///
    /// Returns the error instead of reporting it: the tab cannot reach the status line, and a
    /// caller that silently swallowed this would be claiming the work is recoverable when it
    /// is not.
    pub fn write_draft_if_due(&mut self, now: Instant) -> Option<String> {
        let due = self.draft_due?;
        if now < due {
            return None;
        }
        self.draft_due = None;

        if !self.is_dirty() {
            return None;
        }
        let draft = self.draft.as_ref()?;
        draft.write(self.story()).err()
    }

    /// How long until the pending write, for the frame loop to schedule a repaint.
    #[must_use]
    pub fn draft_wait(&self, now: Instant) -> Option<Duration> {
        self.draft_due.map(|due| due.saturating_duration_since(now))
    }

    /// Remove the working copy and cancel any pending write.
    ///
    /// Called when the file on disk becomes what the document says, and when the author has
    /// agreed to discard. A stale draft would offer back work that was already saved.
    pub fn drop_draft(&mut self) {
        if let Some(draft) = &self.draft {
            draft.discard();
        }
        self.draft_due = None;
    }

    /// Adopt `path` as this document's home after a successful save-as.
    pub fn adopt(&mut self, path: &Path) {
        self.drop_draft();
        self.draft = Draft::beside(path);
        self.assets = AssetPaths::beside(path);
        self.comment_warning_seen = false;
        self.preview_steps.clear();
        self.preview_status.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustory_vn_core::{Exit, Node};

    /// A story with one node, so a tab can be built without touching the disk.
    fn story() -> Story {
        let mut nodes = std::collections::BTreeMap::new();
        nodes.insert(
            NodeId::from("start"),
            Node {
                beats: Vec::new(),
                exit: Exit::End { ending: None },
            },
        );
        Story::new("start", nodes)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rustory-tab-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    /// An editor whose document has a home on disk, so it gets a draft.
    fn saved_tab(dir: &Path, name: &str) -> Tab {
        let path = dir.join(name);
        let mut document = rustory_vn_edit::Document::new(story(), &path);
        document.save_as(&path).expect("writes");
        let editor = Editor::open(&path).expect("opens");
        Tab::new(editor, AssetPaths::beside(&path))
    }

    #[test]
    fn a_new_tab_selects_the_start_node() {
        let dir = scratch("fresh");
        let tab = saved_tab(&dir, "a.ron");
        assert_eq!(tab.selected, Some(NodeId::from("start")));
        assert_eq!(tab.name(), "a.ron");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_untitled_tab_reports_no_file_and_no_draft() {
        let editor = Editor::new_story(None);
        let tab = Tab::new(editor, AssetPaths::from_cwd());
        assert_eq!(tab.name(), "untitled");
        assert!(
            tab.draft.is_none(),
            "a draft with no story to belong to has nowhere to live"
        );
    }

    #[test]
    fn each_tab_writes_its_own_draft() {
        // The bug this shape exists to prevent: one shared deadline writing tab A's story
        // into tab B's recovery file. Two tabs, both dirty, both written, two distinct files.
        let dir = scratch("two-drafts");
        let mut a = saved_tab(&dir, "a.ron");
        let mut b = saved_tab(&dir, "b.ron");

        a.editor
            .apply(&rustory_vn_edit::EditOp::SetTitle {
                title: Some("A title".into()),
            })
            .expect("edits a");
        b.editor
            .apply(&rustory_vn_edit::EditOp::SetTitle {
                title: Some("B title".into()),
            })
            .expect("edits b");

        a.plan_draft();
        b.plan_draft();

        let later = Instant::now() + DRAFT_DELAY + Duration::from_millis(1);
        assert!(a.write_draft_if_due(later).is_none());
        assert!(b.write_draft_if_due(later).is_none());

        let a_text =
            std::fs::read_to_string(a.draft.as_ref().expect("a draft").path()).expect("read a");
        let b_text =
            std::fs::read_to_string(b.draft.as_ref().expect("b draft").path()).expect("read b");

        assert!(a_text.contains("A title"), "{a_text}");
        assert!(
            !a_text.contains("B title"),
            "tab A's recovery copy must not contain tab B's story"
        );
        assert!(b_text.contains("B title"), "{b_text}");
        assert!(!b_text.contains("A title"), "and the other way round");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_draft_write_waits_for_the_debounce() {
        let dir = scratch("debounce");
        let mut tab = saved_tab(&dir, "a.ron");
        tab.editor
            .apply(&rustory_vn_edit::EditOp::SetAuthor {
                author: Some("me".into()),
            })
            .expect("edits");
        tab.plan_draft();

        // Too early: nothing on disk yet.
        assert!(tab.write_draft_if_due(Instant::now()).is_none());
        let draft = tab.draft.as_ref().expect("draft").path().to_path_buf();
        assert!(!draft.exists(), "a write before the deadline is a bug");

        // After the deadline: written once, and the pending write is cleared.
        let later = Instant::now() + DRAFT_DELAY + Duration::from_millis(1);
        assert!(tab.write_draft_if_due(later).is_none());
        assert!(draft.exists());
        assert!(tab.draft_due.is_none(), "the deadline must be consumed");
    }

    #[test]
    fn saving_clears_the_draft_and_discarding_removes_it() {
        let dir = scratch("cleanup");
        let mut tab = saved_tab(&dir, "a.ron");
        tab.editor
            .apply(&rustory_vn_edit::EditOp::SetTitle {
                title: Some("x".into()),
            })
            .expect("edits");
        tab.plan_draft();
        let later = Instant::now() + DRAFT_DELAY + Duration::from_millis(1);
        assert!(tab.write_draft_if_due(later).is_none());
        let draft = tab.draft.as_ref().expect("draft").path().to_path_buf();
        assert!(draft.exists());

        tab.editor.save().expect("saves");
        tab.drop_draft();
        assert!(!draft.exists(), "a saved story has no recovery copy");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn selecting_a_different_node_resets_the_preview_but_reselecting_does_not() {
        let dir = scratch("selection");
        let mut tab = saved_tab(&dir, "a.ron");
        tab.preview_steps.push(Step::Click);

        // Same node: the path stays, because nothing about the position changed.
        Tab::plan_selection(
            &mut tab.selected,
            &mut tab.preview_steps,
            NodeId::from("start"),
        );
        assert_eq!(tab.preview_steps.len(), 1);

        // A different node: the step count belonged to the old one.
        Tab::plan_selection(
            &mut tab.selected,
            &mut tab.preview_steps,
            NodeId::from("elsewhere"),
        );
        assert!(tab.preview_steps.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_selection_falls_back_to_a_node_that_exists() {
        // A delete or a rename can remove the selected node; a panel that renders nothing is
        // worse than one that shows the beginning.
        let dir = scratch("stale");
        let mut tab = saved_tab(&dir, "a.ron");
        tab.selected = Some(NodeId::from("gone"));
        assert_eq!(tab.selected_node(), Some(NodeId::from("start")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
