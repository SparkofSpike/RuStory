//! The editor: applies operations, keeps history, guarantees validity.
//!
//! # The one invariant
//!
//! [`Editor::apply`] returns `Ok` with the change in place, or `Err` with the
//! document **byte-for-byte as it was**. There is no partially applied edit. That
//! matters more for an agent than for a human: a person sees the screen change and
//! reacts, while an agent needs "it failed" to mean nothing happened.
//!
//! It is implemented by snapshotting before the operation and restoring on any
//! failure — including a failure of the story's own validation. Validation therefore
//! does not need to be able to explain which operation broke it; the editor rolls
//! back and reports.
//!
//! # Undo
//!
//! A bounded stack of previous stories. The GUI binds Ctrl+Z, the CLI exposes
//! `--undo`, and neither needs to know how any operation is inverted, because
//! nothing is inverted.

use rustory_core::Story;

use crate::apply;
use crate::document::Document;
use crate::error::EditError;
use crate::op::EditOp;

/// How many undo steps to keep.
///
/// A story is kilobytes, so this is generous; the limit exists so a long session
/// cannot grow without bound, not because memory is tight.
pub const HISTORY_LIMIT: usize = 128;

/// One step that can be undone.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    /// What the operation was, for a tooltip or a log line.
    pub label: String,
}

/// A story plus its editing history.
#[derive(Debug, Clone)]
pub struct Editor {
    document: Document,
    /// States before each applied operation, oldest first.
    past: Vec<Story>,
    /// Labels matching `past`, so undo can say what it is undoing.
    past_labels: Vec<HistoryEntry>,
    /// States undone, for redo, newest first.
    future: Vec<(Story, HistoryEntry)>,
    /// The last operation applied, for undo coalescing.
    ///
    /// Cleared by anything that is not a continuation of the same edit, so two separate
    /// visits to one text field stay two undo steps.
    last_op: Option<EditOp>,
    /// Set when the document differs from what is on disk.
    dirty: bool,
}

impl Editor {
    /// Start editing a document.
    #[must_use]
    pub fn new(document: Document) -> Self {
        Self {
            document,
            past: Vec::new(),
            past_labels: Vec::new(),
            future: Vec::new(),
            last_op: None,
            dirty: false,
        }
    }

    /// Open a story file for editing.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, EditError> {
        Ok(Self::new(Document::open(path)?))
    }

    /// Start a new story that has never been saved.
    ///
    /// The result is immediately editable and immediately valid: one node named
    /// `start` with an `End` exit. A blank document that failed validation would make
    /// every subsequent operation fail, which is a bad first five minutes.
    #[must_use]
    pub fn new_story(title: Option<String>) -> Self {
        use rustory_core::{Exit, Node, NodeId, Story, StoryMeta};
        use std::collections::BTreeMap;

        let start = NodeId::from("start");
        let mut nodes = BTreeMap::new();
        nodes.insert(
            start.clone(),
            Node {
                beats: Vec::new(),
                exit: Exit::End { ending: None },
            },
        );

        Self::new(Document::untitled(Story {
            meta: StoryMeta {
                title,
                author: None,
            },
            start,
            nodes,
        }))
    }

    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }

    #[must_use]
    pub fn story(&self) -> &Story {
        self.document.story()
    }

    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        self.document.path()
    }

    /// Whether there are unsaved changes.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Apply one operation.
    ///
    /// All or nothing. See the module docs.
    pub fn apply(&mut self, op: &EditOp) -> Result<(), EditError> {
        // Snapshot before touching anything: the rollback path must not depend on
        // having succeeded partway.
        let before = self.story().clone();
        let mut working = before.clone();

        apply::apply(&mut working, op)?;

        // The single definition of validity, consulted after every operation.
        working.validate()?;

        // Typing in a field emits one operation per keystroke. Merging a continuation of
        // the same edit means Ctrl+Z steps back over the word rather than the letter,
        // which is what a person expects and what the panel docs always claimed.
        let continuing = op.coalesces_with(self.last_op.as_ref());
        if !continuing {
            self.past.push(before);
            self.past_labels.push(HistoryEntry { label: op.label() });
            if self.past.len() > HISTORY_LIMIT {
                self.past.remove(0);
                self.past_labels.remove(0);
            }
        }

        // A new edit invalidates the redo branch, merged or not: redo past a keystroke
        // would replay a state the user has already typed beyond.
        self.future.clear();
        self.last_op = Some(op.clone());

        self.document.replace_story(working);
        self.dirty = true;
        Ok(())
    }

    /// Apply several operations in order.
    ///
    /// If any fails, the editor is restored to the state before the *batch*, not
    /// before the failing operation: a half-applied batch is exactly the situation
    /// callers cannot reason about. The returned error says which operation failed.
    pub fn apply_all(&mut self, ops: &[EditOp]) -> Result<usize, BatchError> {
        let before = self.story().clone();
        let past_len = self.past.len();
        let labels_len = self.past_labels.len();

        for (index, op) in ops.iter().enumerate() {
            // Each batch entry is its own undo step: a script writer listing ten edits
            // expects ten, not one merged soup.
            self.last_op = None;
            if let Err(source) = self.apply(op) {
                self.rollback_to(before, past_len, labels_len);
                return Err(BatchError { index, source });
            }
        }

        Ok(ops.len())
    }

    fn rollback_to(&mut self, story: Story, past_len: usize, labels_len: usize) {
        self.document.replace_story(story);
        self.past.truncate(past_len);
        self.past_labels.truncate(labels_len);
        self.future.clear();
        self.dirty = true;
    }

    /// Whether undo would do anything.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// Whether redo would do anything.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// What undo would revert, for a menu item or a tooltip.
    #[must_use]
    pub fn undo_label(&self) -> Option<&str> {
        self.past_labels.last().map(|entry| entry.label.as_str())
    }

    /// What redo would reapply.
    #[must_use]
    pub fn redo_label(&self) -> Option<&str> {
        self.future.last().map(|(_, entry)| entry.label.as_str())
    }

    /// Step back one operation.
    ///
    /// Returns the label of what was undone, for a status line.
    pub fn undo(&mut self) -> Option<String> {
        let previous = self.past.pop()?;
        let entry = self.past_labels.pop()?;

        let current = self.story().clone();
        self.future.push((current, entry.clone()));
        self.document.replace_story(previous);
        self.dirty = true;
        // Stepping through history breaks the typing chain: the next keystroke must be a
        // new undo step, not a continuation of the edit that was just undone.
        self.last_op = None;

        Some(entry.label)
    }

    /// Step forward one operation, returning what was reapplied.
    pub fn redo(&mut self) -> Option<String> {
        let (next, entry) = self.future.pop()?;
        let current = self.story().clone();
        self.past.push(current);
        self.past_labels.push(entry.clone());
        self.document.replace_story(next);
        self.dirty = true;
        self.last_op = None;
        self.past_labels.last().map(|entry| entry.label.clone())
    }

    /// Discard history and write the document.
    pub fn save(&mut self) -> Result<(), EditError> {
        self.document.save()?;
        self.dirty = false;
        Ok(())
    }

    /// Write to a new path and adopt it.
    pub fn save_as(&mut self, path: impl AsRef<std::path::Path>) -> Result<(), EditError> {
        self.document.save_as(path)?;
        self.dirty = false;
        Ok(())
    }

    /// Re-read the file from disk, discarding edits and history.
    ///
    /// Exposed for the GUI's "revert" and for an agent that wants to start from a
    /// known state rather than trusting the in-memory document.
    pub fn reload(&mut self) -> Result<(), EditError> {
        self.document = Document::open(self.document.path())?;
        self.past.clear();
        self.past_labels.clear();
        self.future.clear();
        self.last_op = None;
        self.dirty = false;
        Ok(())
    }

    /// Validate without changing anything, returning every problem found.
    ///
    /// `Story::validate` stops at the first fault because during editing one fault at
    /// a time is what you want to fix. For a pre-save check, a complete list is more
    /// useful, so this walks the graph itself.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        let story = self.story();
        let mut found = Vec::new();

        if story.nodes.is_empty() {
            found.push("the story has no nodes".to_owned());
            return found;
        }

        if !story.nodes.contains_key(&story.start) {
            found.push(format!("the start node `{}` does not exist", story.start));
        }

        for (id, node) in &story.nodes {
            match &node.exit {
                rustory_core::Exit::End { .. } => {}
                rustory_core::Exit::Goto { to } => {
                    push_if_dangling(&mut found, story, id, to, "the goto");
                }
                rustory_core::Exit::Choose { choices, .. } => {
                    if choices.is_empty() {
                        found.push(format!(
                            "node `{id}`: a choice exit needs at least one option"
                        ));
                    }
                    for (index, choice) in choices.iter().enumerate() {
                        push_if_dangling(
                            &mut found,
                            story,
                            id,
                            &choice.to,
                            &format!("choice {}", index + 1),
                        );
                    }
                }
                rustory_core::Exit::Gameplay { spec } => {
                    if spec.outcomes.is_empty() {
                        found.push(format!(
                            "node `{id}`: gameplay system `{}` maps no outcomes, so the story cannot continue",
                            spec.system
                        ));
                    }
                    for (outcome, to) in &spec.outcomes {
                        push_if_dangling(
                            &mut found,
                            story,
                            id,
                            to,
                            &format!("outcome `{outcome}`"),
                        );
                    }
                }
            }
        }

        found
    }
}

/// One failed operation inside a batch, with its position.
#[derive(Debug)]
pub struct BatchError {
    /// Zero-based index of the operation that failed.
    pub index: usize,
    /// Why it failed.
    pub source: EditError,
}

impl std::fmt::Display for BatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "operation {} failed: {}", self.index, self.source)
    }
}

impl std::error::Error for BatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn push_if_dangling(
    found: &mut Vec<String>,
    story: &Story,
    from: &rustory_core::NodeId,
    target: &rustory_core::NodeId,
    how: &str,
) {
    if !story.nodes.contains_key(target) {
        found.push(format!(
            "node `{from}`: {how} points at `{target}`, which does not exist"
        ));
    }
}
