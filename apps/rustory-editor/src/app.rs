//! Application state and the frame loop.
//!
//! # The one rule
//!
//! The UI never mutates the story directly. Every interaction produces an
//! [`EditOp`], which is collected during layout and applied afterwards. Two reasons:
//!
//! * Borrowing — the story is borrowed by the panels that display it.
//! * Correctness — routing through `EditOp` means the GUI cannot reach a state the
//!   CLI cannot, which is what keeps "an agent can do anything a person can" true
//!   rather than aspirational.

use std::path::PathBuf;

use rustory_core::NodeId;
use rustory_edit::prelude::*;
use rustory_edit::{Document, Editor};

use crate::panels;
use crate::preview::{AssetPaths, Preview};

pub struct EditorApp {
    /// `None` only before a document exists.
    pub editor: Option<Editor>,
    pub selected: Option<NodeId>,
    /// A pending "new node id" string, so the field can hold invalid text while the
    /// user types. Parsed into a [`NodeId`] only on submit.
    pub new_node_id: String,
    /// Same idea for renaming.
    pub rename_buffer: String,
    pub renaming: Option<NodeId>,

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
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, story: Option<String>) -> Self {
        let mut app = Self {
            editor: None,
            selected: None,
            new_node_id: String::new(),
            rename_buffer: String::new(),
            renaming: None,
            preview_step: 0,
            preview: Preview::new(),
            assets: AssetPaths::from_cwd(),
            preview_status: String::new(),
            status: "Ready".to_owned(),
            status_is_error: false,
            pending: Vec::new(),
        };

        app.preview.ensure_ready(cc);

        if let Some(path) = story {
            app.open_path(PathBuf::from(path));
        }

        app
    }

    /// Load a story, replacing whatever was open.
    pub fn open_path(&mut self, path: PathBuf) {
        match Editor::open(&path) {
            Ok(editor) => {
                self.selected = Some(editor.story().start.clone());
                self.editor = Some(editor);
                self.preview_step = 0;
                // Art is looked up beside the story, not beside the process. Opening a
                // project from a different directory must not silently lose its images.
                self.assets = AssetPaths::beside(&path);
                self.set_status(format!("Opened {}", path.display()), false);
            }
            Err(err) => self.set_status(err.to_string(), true),
        }
    }

    /// Load a document that has not been saved anywhere yet.
    ///
    /// Not reached from the UI yet: starting a new story needs a way to choose where
    /// it will be saved, which means a save dialog, which is a known gap. Kept because
    /// the function is what that dialog will call.
    #[allow(dead_code)]
    pub fn adopt(&mut self, document: Document) {
        self.selected = Some(document.story().start.clone());
        self.editor = Some(Editor::new(document));
        self.preview_step = 0;
        self.set_status("New story".to_owned(), false);
    }

    pub fn set_status(&mut self, message: impl Into<String>, is_error: bool) {
        self.status = message.into();
        self.status_is_error = is_error;
    }

    /// Apply everything the UI asked for this frame.
    fn flush(&mut self) {
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

    fn save(&mut self) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let path = editor.path().display().to_string();
        match editor.save() {
            Ok(()) => self.set_status(format!("Saved {path}"), false),
            Err(err) => self.set_status(err.to_string(), true),
        }
    }

    // Public wrappers for the panels. The panels do not mutate state directly; they
    // ask the app to, which is also where the status line is updated.

    pub fn save_pending(&mut self) {
        self.save();
    }

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
        // `ui.ctx()` is the context for this pass; reading input here keeps the
        // handler in one place instead of scattering `input(...)` per panel.
        let (undo, redo, save) = ui.ctx().input(|input| {
            (
                input.modifiers.command && input.key_pressed(egui::Key::Z),
                input.modifiers.command && input.key_pressed(egui::Key::Y),
                input.modifiers.command && input.key_pressed(egui::Key::S),
            )
        });
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if save {
            self.save();
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
