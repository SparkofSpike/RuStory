//! Application state and the frame loop.
//!
//! # The one rule
//!
//! The UI never mutates the story directly. Every interaction produces either an
//! [`EditOp`] or a [`UiAction`], which is collected during layout and handled
//! afterwards. Two reasons:
//!
//! * Borrowing — the story is borrowed by the panels that display it.
//! * Correctness — routing through `EditOp` means the GUI cannot reach a state the
//!   CLI cannot, which is what keeps "an agent can do anything a person can" true
//!   rather than aspirational.
//!
//! [`UiAction`] is the other half: file operations are not edits. Open, save, and new
//! change *which document is being edited*, not its contents, and they need to talk to
//! the filesystem and to dialogs. Keeping them separate from `EditOp` means the
//! operation layer stays free of I/O policy.
//!
//! # The window and the document
//!
//! [`EditorApp`] is the window: several [`Tab`]s, one shared renderer, one status line, one
//! recent list. Everything that is about *a story* lives on a tab, and everything that is
//! about *the window* lives here. The split is not cosmetic — a shared renderer per tab would
//! be one GPU device per open file, and a shared draft deadline per window would write one
//! story's recovery copy over another's.

use std::path::{Path, PathBuf};

use rustory_vn_core::NodeId;
use rustory_vn_edit::Draft;
use rustory_vn_edit::Editor;

use crate::dialogs;
use crate::panels;
use crate::preview::{AssetPaths, Preview};
use crate::recent::RecentFiles;
use crate::tab::Tab;

/// A file-level request from the UI.
///
/// Deferred rather than executed inline so it can be handled after the panels stop
/// borrowing the document — the same reason edits are deferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    Open,
    /// Open one specific file, from the recent list.
    OpenAt(PathBuf),
    New,
    Save,
    /// Save the tab at this index. The index is carried because a save reads a dialog and
    /// writes a file, and the active tab may have changed by the time it runs.
    SaveAs(usize),
    /// Close the tab at this index, after asking about unsaved work.
    Close(usize),
    /// Empty the recent list.
    ClearRecent,
}

pub struct EditorApp {
    /// Open documents, in tab order. May be empty — closing the last tab is allowed, and an
    /// empty window offering "open a story" is better than one that refuses to close.
    pub tabs: Vec<Tab>,
    /// Which tab the panels and the shortcuts act on. Always a valid index into `tabs`.
    pub active: usize,
    /// The one GPU renderer, shared by every tab.
    pub preview: Preview,

    /// Last thing that happened, shown in the status bar.
    pub status: String,
    pub status_is_error: bool,

    /// File-level requests produced during this frame.
    pub pending_actions: Vec<UiAction>,

    /// Set while the close guard is deciding whether a close may proceed.
    pub close_confirmed: bool,

    /// Files opened recently, newest first. Persisted between runs so a person who
    /// edits one chapter every evening does not have to find it in a dialog again.
    pub recent: RecentFiles,

    /// Whether the shortcut sheet is open.
    pub show_help: bool,
}

/// What a close decision was about, so the caller knows what to say afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    /// Nothing had unsaved work; it is closed.
    Closed,
    /// The author said no. Nothing changed.
    Kept,
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, story: Option<String>) -> Self {
        let mut app = Self {
            tabs: Vec::new(),
            active: 0,
            preview: Preview::new(),
            status: "Ready".to_owned(),
            status_is_error: false,
            pending_actions: Vec::new(),
            close_confirmed: false,
            recent: RecentFiles::load(),
            show_help: false,
        };

        // A list that mentions files deleted outside the editor is worse than a short
        // one: every entry in the menu has to be a file that opens.
        if app.recent.prune() {
            let _ = app.recent.save();
        }

        app.preview.ensure_ready(cc);

        match story {
            Some(path) => app.open_path(PathBuf::from(path)),
            // Starting with a blank story rather than an empty window: an editor that
            // shows nothing on launch makes the user find the File menu before they can
            // do anything at all.
            None => app.new_story(),
        }

        app
    }

    // --- the active tab ---------------------------------------------------

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    /// The index of the tab editing `path`, if it is already open.
    ///
    /// Comparing canonical-ish paths rather than raw ones: opening `stories/demo.ron` after
    /// `E:\Coding\RuStory\stories\demo.ron` must find the tab, not make a second one for the
    /// same file. Two tabs on one file would each hold their own copy, and the last save
    /// would silently win.
    #[must_use]
    pub fn find_tab(&self, path: &Path) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.editor.document().has_file() && same_path(tab.path(), path))
    }

    // --- opening ----------------------------------------------------------

    /// Load a story into a new tab, or switch to it if it is already open.
    ///
    /// A tab that was already open is not reloaded: the document in memory may have unsaved
    /// edits, and silently discarding them because the file appeared in the recent list would
    /// be the same data loss the close guard exists to prevent.
    pub fn open_path(&mut self, path: PathBuf) {
        if let Some(index) = self.find_tab(&path) {
            self.active = index;
            self.set_status(format!("Switched to {}", path.display()), false);
            return;
        }

        // Before reading the file: a draft left by a crash is offered, not applied. If the
        // author takes it, the file itself is rewritten, which is why the dialog spells out
        // both outcomes.
        self.settle_draft(&path);

        match Editor::open(&path) {
            Ok(editor) => {
                // Art is looked up beside the story, not beside the process. Opening a
                // project from a different directory must not silently lose its images.
                let tab = Tab::new(editor, AssetPaths::beside(&path));
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                self.remember(&path);
                self.set_status(format!("Opened {}", path.display()), false);
            }
            Err(err) => {
                // A listed file that is gone is a stale entry, not a mystery: drop it so
                // the next attempt is not made at the same ghost.
                if self.recent.forget(&path) {
                    let _ = self.recent.save();
                }
                self.set_status(err.to_string(), true);
            }
        }
    }

    /// Begin a new story in a new tab.
    pub fn new_story(&mut self) {
        let editor = Editor::new_story(Some("Untitled".to_owned()));
        let tab = Tab::new(editor, AssetPaths::from_cwd());
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.set_status("New story — save it to choose a location", false);
    }

    /// Add a path to the recent list and persist the change.
    ///
    /// A write failure does not deserve a dialog: the menu still works for this session,
    /// and the next save tries the same directory again.
    fn remember(&mut self, path: &Path) {
        if self.recent.record(path) {
            let _ = self.recent.save();
        }
    }

    /// Deal with a draft left beside `path`, if there is one.
    ///
    /// Three outcomes: none there, recovered, or discarded. A draft that cannot be parsed is
    /// discarded silently — the author still has the file, and offering a document that will
    /// not open would turn a recovery into a puzzle.
    fn settle_draft(&mut self, path: &Path) {
        let Some(draft) = Draft::beside(path) else {
            return;
        };
        if !draft.exists() {
            return;
        }

        let Some(story) = draft.read() else {
            draft.discard();
            return;
        };

        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );

        if !dialogs::confirm_recover(&name) {
            draft.discard();
            return;
        }

        match draft.recover_into(&story, path) {
            Ok(()) => self.set_status(format!("Recovered unsaved changes to {name}"), false),
            Err(err) => self.set_status(format!("Could not recover: {err}"), true),
        }
    }

    // --- closing ----------------------------------------------------------

    /// Close a tab, asking first if it has unsaved work.
    pub fn close_tab(&mut self, index: usize) -> CloseOutcome {
        if index >= self.tabs.len() {
            return CloseOutcome::Closed;
        }
        if !self.confirm_discard(index) {
            return CloseOutcome::Kept;
        }
        self.remove_tab(index);
        CloseOutcome::Closed
    }

    /// Remove a tab and make the active index sensible again.
    ///
    /// Split from `close_tab` so the index arithmetic can be tested without a dialog. That
    /// arithmetic is where an off-by-one hides: the tab that was to the right slides into the
    /// closed index, and an active index past the end has to land on the new last tab.
    fn remove_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let mut tab = self.tabs.remove(index);
        tab.drop_draft();

        // Keep the selection adjacent rather than jumping to the start: closing tab 3 of 5
        // should land on what is now tab 3, which is the tab that was next to it.
        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active > index {
            self.active -= 1;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
    }

    /// Whether one tab may be thrown away, asking once if it has unsaved work.
    fn confirm_discard(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get(index) else {
            return true;
        };
        if !tab.is_dirty() {
            return true;
        }
        let name = tab.name();

        if dialogs::confirm_discard(&name) {
            // The author has just said the work can go, so this tab's recovery copy goes with
            // it. A draft left behind would be offered back on the next open, which is the
            // opposite of what was just decided.
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.drop_draft();
            }
            true
        } else {
            self.set_status(format!("Kept `{name}` open"), false);
            false
        }
    }

    /// Whether the window may close: every dirty tab is asked about, in order.
    ///
    /// All of them, not just the active one. A window with three unsaved chapters and a
    /// guard that only looked at the visible one would lose the other two without a word.
    pub fn confirm_close_all(&mut self) -> bool {
        let dirty: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.is_dirty())
            .map(|(index, _)| index)
            .collect();

        for index in dirty {
            if !self.confirm_discard(index) {
                // Ask about the rest later: the author has already said "not this one", and
                // that is enough to stop the close.
                return false;
            }
        }
        true
    }

    // --- status -----------------------------------------------------------

    pub fn set_status(&mut self, message: impl Into<String>, is_error: bool) {
        self.status = message.into();
        self.status_is_error = is_error;
    }

    // --- per-frame work ---------------------------------------------------

    /// Apply everything the UI asked for this frame.
    ///
    /// Every tab's queue, not only the active one: an operation produced in tab A and then
    /// followed by a switch to tab B would otherwise be applied to B, naming a node that may
    /// not exist there.
    fn flush(&mut self) {
        let actions = std::mem::take(&mut self.pending_actions);
        for action in actions {
            self.handle(action);
        }

        let mut last_status: Option<(String, bool)> = None;

        for tab in &mut self.tabs {
            if tab.pending.is_empty() {
                continue;
            }
            let ops = std::mem::take(&mut tab.pending);
            let mut selection_after: Option<Option<NodeId>> = None;

            for op in ops {
                let label = op.label();
                match tab.editor.apply(&op) {
                    Ok(()) => {
                        last_status = Some((label, false));
                        // Structural edits can invalidate the selection.
                        if op.changes_structure()
                            && let Some(selected) = &tab.selected
                            && !tab.story().nodes.contains_key(selected)
                        {
                            selection_after = Some(tab.story().nodes.keys().next().cloned());
                        }
                    }
                    Err(err) => {
                        last_status = Some((err.to_string(), true));
                        // Stop at the first failure: later operations were written assuming
                        // this one succeeded.
                        break;
                    }
                }
            }

            if let Some(selection) = selection_after {
                tab.selected = selection;
            }
            tab.plan_draft();
        }

        if let Some((message, is_error)) = last_status {
            self.set_status(message, is_error);
        }
    }

    fn handle(&mut self, action: UiAction) {
        match action {
            UiAction::Open => {
                if let Some(path) = dialogs::open_story() {
                    self.open_path(path);
                }
            }
            UiAction::OpenAt(path) => self.open_path(path),
            UiAction::New => self.new_story(),
            UiAction::Save => self.save(),
            UiAction::SaveAs(index) => self.save_as(index),
            UiAction::Close(index) => {
                self.close_tab(index);
            }
            UiAction::ClearRecent => {
                if self.recent.clear() {
                    let _ = self.recent.save();
                }
                self.set_status("Cleared the recent list", false);
            }
        }
    }

    // --- saving -----------------------------------------------------------

    /// Save the active tab, asking for a path when the document has never had one.
    pub fn save(&mut self) {
        let Some(index) = self.tabs.get(self.active).map(|_| self.active) else {
            return;
        };
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if !tab.editor.document().has_file() {
            self.pending_actions.push(UiAction::SaveAs(index));
            return;
        }

        let path = tab.path().to_path_buf();
        let name = tab.name();

        // Before the first overwrite of this document: saving rewrites the file from the data
        // model, and comments are not part of one.
        let needs_warning =
            !tab.comment_warning_seen && tab.editor.document().source_has_comments();
        if needs_warning && !dialogs::confirm_rewrite(&name) {
            self.set_status("Save cancelled; the file is unchanged", false);
            return;
        }
        if needs_warning && let Some(tab) = self.tabs.get_mut(index) {
            tab.comment_warning_seen = true;
        }

        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        match tab.editor.save() {
            Ok(()) => {
                // The file is now what the document says, so the recovery copy is stale by
                // definition. Leaving it would offer the author work they already saved.
                tab.drop_draft();
                self.set_status(format!("Saved {}", path.display()), false);
            }
            Err(err) => self.set_status(err.to_string(), true),
        }
    }

    fn save_as(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let suggested = tab.path().to_path_buf();

        let Some(path) = dialogs::save_story_as(Some(&suggested)) else {
            return;
        };

        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        match tab.editor.save_as(&path) {
            Ok(()) => {
                tab.adopt(&path);
                let name = path.display().to_string();
                // The art may live somewhere else now, so the cached textures and frame are
                // both stale. `reset` drops those and keeps the renderer — replacing the
                // whole `Preview` would discard the GPU device it borrows from eframe.
                self.preview.reset();
                self.remember(&path);
                self.set_status(format!("Saved {name}"), false);
            }
            Err(err) => self.set_status(err.to_string(), true),
        }
    }

    // --- undo -------------------------------------------------------------

    fn undo(&mut self) {
        if let Some(tab) = self.active_tab_mut()
            && let Some(label) = tab.editor.undo()
        {
            tab.plan_draft();
            self.set_status(format!("undo: {label}"), false);
        }
    }

    fn redo(&mut self) {
        if let Some(tab) = self.active_tab_mut()
            && let Some(label) = tab.editor.redo()
        {
            tab.plan_draft();
            self.set_status(format!("redo: {label}"), false);
        }
    }

    // Public wrappers for the panels. Panels do not mutate state directly; they ask the
    // app to, which is also where the status line is updated.

    pub fn undo_pending(&mut self) {
        self.undo();
    }

    pub fn redo_pending(&mut self) {
        self.redo();
    }

    // --- the frame --------------------------------------------------------

    /// Write every tab's recovery copy whose debounce has expired.
    ///
    /// Every tab, not just the visible one: a document sitting in a background tab has the
    /// same claim to being recovered as the one on screen, and the whole point of a draft is
    /// that the author was not looking at the window when the process died.
    fn write_drafts(&mut self) {
        let now = std::time::Instant::now();
        let mut failure: Option<String> = None;
        for tab in &mut self.tabs {
            if let Some(err) = tab.write_draft_if_due(now) {
                failure = Some(err);
            }
        }
        if let Some(err) = failure {
            self.set_status(format!("Could not write the recovery copy: {err}"), true);
        }
    }

    /// The soonest pending draft write, so the frame loop can schedule a repaint.
    ///
    /// egui only redraws on input by default, and a debounced write has to happen while the
    /// author is sitting still — exactly when nothing is repainting.
    fn next_draft_wait(&self) -> Option<std::time::Duration> {
        let now = std::time::Instant::now();
        self.tabs.iter().filter_map(|tab| tab.draft_wait(now)).min()
    }
}

/// Whether two paths name the same file, as far as a person means.
///
/// Not `canonicalize`: the file may not exist yet (an untitled save), and on Windows
/// canonicalising adds a `\\?\` prefix that makes the comparison fail for the very case it is
/// supposed to catch. Case-insensitive on Windows, because `Story.RON` and `story.ron` are one
/// file there.
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(windows)]
    {
        a.to_string_lossy().replace('/', "\\").to_lowercase()
            == b.to_string_lossy().replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Shortcuts first: they must work regardless of which panel has focus, and they act
        // on the active tab — the one the author is looking at.
        //
        // Except while a text field has the keyboard. `Ctrl+Z` in a field is the field's undo,
        // and `Ctrl+W` is delete-word in enough editors that closing a tab under the author's
        // hands would be a nasty surprise. File verbs (save, open, new) are safe either way:
        // no text field gives them a meaning.
        //
        // `text_edit_focused` rather than "anything has focus": with a button focused, undo is
        // still the document's, and taking it away would make the shortcut feel broken.
        let editing_text = ui.ctx().text_edit_focused();
        let shortcuts = ui.ctx().input(|input| {
            let command = input.modifiers.command;
            let shift = input.modifiers.shift;
            (
                command && !shift && !editing_text && input.key_pressed(egui::Key::Z),
                command && !shift && !editing_text && input.key_pressed(egui::Key::Y),
                command && shift && input.key_pressed(egui::Key::S),
                command && !shift && input.key_pressed(egui::Key::S),
                command && input.key_pressed(egui::Key::O),
                command && !shift && input.key_pressed(egui::Key::N),
                input.key_pressed(egui::Key::F1),
                command && !shift && !editing_text && input.key_pressed(egui::Key::W),
            )
        });
        let (undo, redo, save_as, save, open, new, help, close) = shortcuts;

        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if save_as {
            self.pending_actions.push(UiAction::SaveAs(self.active));
        }
        if save {
            self.pending_actions.push(UiAction::Save);
        }
        if open {
            self.pending_actions.push(UiAction::Open);
        }
        if new {
            self.pending_actions.push(UiAction::New);
        }
        if help {
            self.show_help = !self.show_help;
        }
        if close {
            self.pending_actions.push(UiAction::Close(self.active));
        }

        // Close guard. Runs before layout so a cancel takes effect this frame.
        if ui.ctx().input(|input| input.viewport().close_requested()) && !self.close_confirmed {
            if self.confirm_close_all() {
                self.close_confirmed = true;
            } else {
                // Refuse the close, and undo the viewport's own decision so the window
                // stays open rather than vanishing on the next pass.
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }

        panels::top_bar(self, ui);
        panels::tab_bar(self, ui);
        panels::status_bar(self, ui);
        panels::node_list(self, ui);
        panels::inspector(self, ui);
        panels::beat_editor(self, ui);
        panels::help_window(self, ui);

        // Everything the panels asked for lands here, after layout, so the story is
        // not borrowed while it is being changed.
        self.flush();

        // The working copies. Scheduled before the write so a frame that only waits still
        // repaints when the deadline arrives.
        if let Some(wait) = self.next_draft_wait() {
            ui.ctx().request_repaint_after(wait);
            self.write_drafts();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app with `count` untitled tabs and no GPU, which is all the index arithmetic needs.
    fn app_with(count: usize) -> EditorApp {
        let mut app = EditorApp {
            tabs: Vec::new(),
            active: 0,
            preview: Preview::new(),
            status: String::new(),
            status_is_error: false,
            pending_actions: Vec::new(),
            close_confirmed: false,
            recent: RecentFiles::default(),
            show_help: false,
        };
        for _ in 0..count {
            let editor = Editor::new_story(None);
            app.tabs.push(Tab::new(editor, AssetPaths::from_cwd()));
        }
        app
    }

    #[test]
    fn closing_a_tab_to_the_left_keeps_the_same_document_active() {
        let mut app = app_with(3);
        app.active = 2;

        app.remove_tab(0);

        // The document that was at index 2 is now at index 1, and it is still the one the
        // author is looking at — not the tab that took index 2.
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 1);
    }

    #[test]
    fn closing_the_active_tab_lands_on_the_one_that_moved_into_its_place() {
        let mut app = app_with(3);
        app.active = 1;

        app.remove_tab(1);

        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 1, "the tab that slid left takes the focus");
    }

    #[test]
    fn closing_the_last_tab_pulls_the_selection_back() {
        let mut app = app_with(2);
        app.active = 1;

        app.remove_tab(1);

        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active, 0, "there is no index 1 left to be active");
    }

    #[test]
    fn closing_the_only_tab_leaves_an_empty_window() {
        let mut app = app_with(1);

        app.remove_tab(0);

        assert!(app.tabs.is_empty());
        assert_eq!(app.active, 0, "an empty window still has a usable index");
        assert!(app.active_tab_mut().is_none());
    }

    #[test]
    fn closing_a_tab_that_is_not_there_does_nothing() {
        let mut app = app_with(1);
        app.remove_tab(7);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active, 0);
    }

    #[test]
    fn opening_a_path_that_is_already_open_switches_instead_of_duplicating() {
        // Two tabs on one file would each hold a copy, and the last save would silently win.
        let dir = std::env::temp_dir().join(format!("rustory-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let path = dir.join("story.ron");

        let mut document =
            rustory_vn_edit::Document::new(Editor::new_story(None).story().clone(), &path);
        document.save_as(&path).expect("writes");

        let mut app = app_with(0);
        app.open_path(path.clone());
        assert_eq!(app.tabs.len(), 1);
        app.open_path(path.clone());
        assert_eq!(app.tabs.len(), 1, "the same file must not open twice");
        assert_eq!(app.active, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_path_ignores_case_and_separators_on_windows() {
        assert!(same_path(
            Path::new("stories/a.ron"),
            Path::new("stories/a.ron")
        ));
        #[cfg(windows)]
        {
            assert!(same_path(
                Path::new("stories/a.ron"),
                Path::new("stories\\A.RON")
            ));
            assert!(!same_path(
                Path::new("stories/a.ron"),
                Path::new("stories/b.ron")
            ));
        }
    }
}
