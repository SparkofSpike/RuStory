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

use std::path::{Path, PathBuf};

use rustory_core::NodeId;
use rustory_edit::Editor;
use rustory_edit::prelude::*;

use crate::dialogs;
use crate::panels;
use crate::preview::{AssetPaths, Preview};
use crate::recent::RecentFiles;

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
    SaveAs,
    /// Empty the recent list.
    ClearRecent,
}

pub struct EditorApp {
    /// `None` only before a document exists.
    pub editor: Option<Editor>,
    pub selected: Option<NodeId>,
    /// A pending "new node id" string, so the field can hold invalid text while the
    /// user types. Parsed into a [`NodeId`] only on submit.
    pub new_node_id: String,
    /// Same idea for renaming: holds text until the author submits it.
    pub rename_buffer: String,
    /// Buffer for a new gameplay outcome key, so the field can hold text that is not
    /// yet a valid key.
    pub new_outcome: String,
    /// Node-list filter. Not an edit: it changes what is shown, not what is stored, so
    /// it takes no undo step.
    pub node_filter: String,
    /// Whether the story settings form is expanded.
    pub show_story_settings: bool,
    /// How many interactions the preview has applied. The preview is a real
    /// `Narrative`, so this is a genuine position in the story rather than a slide.
    pub preview_step: usize,
    pub preview: Preview,
    /// Where to look for art, derived from the open file's location.
    pub assets: AssetPaths,
    /// Status line of the last preview render.
    pub preview_status: String,

    /// Last thing that happened, shown in the status bar.
    pub status: String,
    /// Whether the status is an error, so it can be coloured.
    pub status_is_error: bool,

    /// Operations produced during this frame, applied before drawing the next.
    pub pending: Vec<EditOp>,
    /// File-level requests produced during this frame.
    pub pending_actions: Vec<UiAction>,

    /// Set while the close guard is deciding whether a close may proceed.
    pub close_confirmed: bool,

    /// Files opened recently, newest first. Persisted between runs so a person who
    /// edits one chapter every evening does not have to find it in a dialog again.
    pub recent: RecentFiles,
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, story: Option<String>) -> Self {
        let mut app = Self {
            editor: None,
            selected: None,
            new_node_id: String::new(),
            rename_buffer: String::new(),
            new_outcome: String::new(),
            node_filter: String::new(),
            show_story_settings: false,
            preview_step: 0,
            preview: Preview::new(),
            assets: AssetPaths::from_cwd(),
            preview_status: String::new(),
            status: "Ready".to_owned(),
            status_is_error: false,
            pending: Vec::new(),
            pending_actions: Vec::new(),
            close_confirmed: false,
            recent: RecentFiles::load(),
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

    /// Load a story, replacing whatever was open.
    ///
    /// Refuses when there is unsaved work, after asking. Losing a scene to a misclick
    /// is the one failure an editor must not have.
    pub fn open_path(&mut self, path: PathBuf) {
        if !self.may_discard() {
            return;
        }

        match Editor::open(&path) {
            Ok(editor) => {
                self.selected = Some(editor.story().start.clone());
                self.editor = Some(editor);
                self.preview_step = 0;
                self.preview.reset();
                // Art is looked up beside the story, not beside the process. Opening a
                // project from a different directory must not silently lose its images.
                self.assets = AssetPaths::beside(&path);
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

    /// Add a path to the recent list and persist the change.
    ///
    /// A write failure does not deserve a dialog: the menu still works for this session,
    /// and the next save tries the same directory again.
    fn remember(&mut self, path: &Path) {
        if self.recent.record(path) {
            let _ = self.recent.save();
        }
    }

    /// Begin a new story.
    pub fn new_story(&mut self) {
        if !self.may_discard() {
            return;
        }

        let editor = Editor::new_story(Some("Untitled".to_owned()));
        self.selected = Some(editor.story().start.clone());
        self.editor = Some(editor);
        self.preview_step = 0;
        self.preview.reset();
        self.assets = AssetPaths::from_cwd();
        self.set_status("New story — save it to choose a location", false);
    }

    /// Whether unsaved work may be thrown away.
    ///
    /// A clean document never asks. A dirty one asks once, and a "no" stops whatever
    /// was about to happen.
    fn may_discard(&mut self) -> bool {
        let Some(editor) = self.editor.as_ref() else {
            return true;
        };
        if !editor.is_dirty() {
            return true;
        }

        let name = editor.path().file_name().map_or_else(
            || "untitled".to_owned(),
            |name| name.to_string_lossy().to_string(),
        );

        if dialogs::confirm_discard(&name) {
            true
        } else {
            self.set_status("Kept the unsaved story open", false);
            false
        }
    }

    /// Select a node for editing.
    ///
    /// Resets the preview step when the selection actually changes: a step count belongs
    /// to the position it was counted at, and carrying it into a different node shows a
    /// frame that has nothing to do with what the author just clicked.
    pub fn select(&mut self, id: NodeId) {
        if self.selected.as_ref() != Some(&id) {
            self.preview_step = 0;
        }
        self.selected = Some(id);
    }

    pub fn set_status(&mut self, message: impl Into<String>, is_error: bool) {
        self.status = message.into();
        self.status_is_error = is_error;
    }

    /// Apply everything the UI asked for this frame.
    fn flush(&mut self) {
        let actions = std::mem::take(&mut self.pending_actions);
        for action in actions {
            self.handle(action);
        }

        let ops = std::mem::take(&mut self.pending);
        if ops.is_empty() {
            return;
        }

        let Some(editor) = self.editor.as_mut() else {
            return;
        };

        // Collected rather than reported inline: `set_status` borrows all of `self`,
        // which would conflict with the `editor` borrow held by the loop.
        let mut last_status: Option<(String, bool)> = None;
        let mut selection_after: Option<Option<NodeId>> = None;

        for op in ops {
            let label = op.label();
            match editor.apply(&op) {
                Ok(()) => {
                    last_status = Some((label, false));
                    // Structural edits can invalidate the selection.
                    if op.changes_structure()
                        && let Some(selected) = &self.selected
                        && !editor.story().nodes.contains_key(selected)
                    {
                        selection_after = Some(editor.story().nodes.keys().next().cloned());
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
            self.selected = selection;
        }
        if let Some((message, is_error)) = last_status {
            self.set_status(message, is_error);
        }
    }

    fn handle(&mut self, action: UiAction) {
        match action {
            UiAction::Open => {
                // The dialog runs before `may_discard` would, so a cancelled dialog does
                // not trigger an unsaved-changes prompt.
                if let Some(path) = dialogs::open_story() {
                    self.open_path(path);
                }
            }
            UiAction::OpenAt(path) => self.open_path(path),
            UiAction::New => self.new_story(),
            UiAction::Save => self.save(),
            UiAction::SaveAs => self.save_as(),
            UiAction::ClearRecent => {
                if self.recent.clear() {
                    let _ = self.recent.save();
                }
                self.set_status("Cleared the recent list", false);
            }
        }
    }

    /// Save, asking for a path when the document has never had one.
    pub fn save(&mut self) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let path = editor.path().to_path_buf();
        let has_file = editor.document().has_file();

        if !has_file {
            self.save_as();
            return;
        }

        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        match editor.save() {
            Ok(()) => self.set_status(format!("Saved {}", path.display()), false),
            Err(err) => self.set_status(err.to_string(), true),
        }
    }

    fn save_as(&mut self) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let suggested = editor.path().to_path_buf();

        let Some(path) = dialogs::save_story_as(Some(&suggested)) else {
            return;
        };

        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        match editor.save_as(&path) {
            Ok(()) => {
                // The assets move with the story: saving a chapter into a project
                // directory should look for that project's art.
                self.assets = AssetPaths::beside(&path);
                self.remember(&path);
                self.set_status(format!("Saved {}", path.display()), false);
            }
            Err(err) => self.set_status(err.to_string(), true),
        }
    }

    fn undo(&mut self) {
        if let Some(editor) = self.editor.as_mut()
            && let Some(label) = editor.undo()
        {
            self.set_status(format!("undo: {label}"), false);
        }
    }

    fn redo(&mut self) {
        if let Some(editor) = self.editor.as_mut()
            && let Some(label) = editor.redo()
        {
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
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Shortcuts first: they must work regardless of which panel has focus.
        let shortcuts = ui.ctx().input(|input| {
            let command = input.modifiers.command;
            let shift = input.modifiers.shift;
            (
                command && !shift && input.key_pressed(egui::Key::Z),
                command && !shift && input.key_pressed(egui::Key::Y),
                command && shift && input.key_pressed(egui::Key::S),
                command && !shift && input.key_pressed(egui::Key::S),
                command && input.key_pressed(egui::Key::O),
                command && input.key_pressed(egui::Key::N),
            )
        });
        let (undo, redo, save_as, save, open, new) = shortcuts;

        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if save_as {
            self.pending_actions.push(UiAction::SaveAs);
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

        // Close guard. Runs before layout so a cancel takes effect this frame.
        if ui.ctx().input(|input| input.viewport().close_requested()) && !self.close_confirmed {
            if self.may_discard() {
                self.close_confirmed = true;
            } else {
                // Refuse the close, and undo the viewport's own decision so the window
                // stays open rather than vanishing on the next pass.
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }

        panels::top_bar(self, ui);
        panels::status_bar(self, ui);
        panels::node_list(self, ui);
        panels::inspector(self, ui);
        panels::beat_editor(self, ui);

        // Everything the panels asked for lands here, after layout, so the story is
        // not borrowed while it is being changed.
        self.flush();
    }
}
