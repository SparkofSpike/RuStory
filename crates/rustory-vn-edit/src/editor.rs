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

use rustory_vn_core::{NodeId, Story};

use crate::apply::{self, Reference};
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

/// One point on the editor's timeline, as a history panel lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelinePoint {
    /// 0 is the document as it was opened; `n` is the state after the nth operation.
    pub index: usize,
    /// What produced this state. Empty for the opening state, which no operation made.
    pub label: String,
    /// Whether this is where the editor is now.
    pub current: bool,
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
        use rustory_vn_core::{Exit, Node, NodeId, Story, StoryMeta};
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

        let mut story = Story::new(start, nodes);
        story.meta = StoryMeta {
            title,
            author: None,
        };
        Self::new(Document::untitled(story))
    }

    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }

    #[must_use]
    pub fn story(&self) -> &Story {
        self.document.story()
    }

    /// Every reference to `node`, in a stable order.
    ///
    /// The same function deletion and rename consult, so a front-end that lists these
    /// before a delete lists exactly what would block it. Exposed rather than
    /// reimplemented because two answers to "what points here" is how a UI starts
    /// disagreeing with the operation it is guarding.
    #[must_use]
    pub fn referrers(&self, node: &NodeId) -> Vec<Reference> {
        apply::referrers_to(self.story(), node)
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

    /// Every problem in the current document. See [`problems_of`].
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        problems_of(self.story())
    }

    /// The timeline: every state this editor can jump to, oldest first.
    ///
    /// Index 0 is the document as it was opened; index `n` is the state after the nth
    /// operation. States undone are included — a creator who has stepped back five times
    /// wants to see what is still ahead — so the list can be longer than the undo stack.
    ///
    /// Once [`HISTORY_LIMIT`] is reached the oldest states are dropped, and index 0 becomes
    /// the oldest state still reachable rather than the file as it was opened. A panel that
    /// says "step 3" is telling the truth about what it can do, not about the session's
    /// entire past.
    #[must_use]
    pub fn timeline(&self) -> Vec<TimelinePoint> {
        let mut points = Vec::with_capacity(self.past.len() + self.future.len() + 1);
        points.push(TimelinePoint {
            index: 0,
            label: String::new(),
            current: self.past.is_empty(),
        });
        for (n, entry) in self.past_labels.iter().enumerate() {
            points.push(TimelinePoint {
                index: n + 1,
                label: entry.label.clone(),
                current: n + 1 == self.past.len(),
            });
        }
        for (n, (_, entry)) in self.future.iter().rev().enumerate() {
            points.push(TimelinePoint {
                index: self.past.len() + 1 + n,
                label: entry.label.clone(),
                current: false,
            });
        }
        points
    }

    /// Where the editor is on its timeline, for highlighting the current row.
    #[must_use]
    pub fn timeline_position(&self) -> usize {
        self.past.len()
    }

    /// Jump to a point on the timeline.
    ///
    /// This is [`Editor::undo`] or [`Editor::redo`] repeated, not a second history mechanism.
    /// One stack of states, one way along it — which is why a jump cannot disagree with the
    /// undo button about what the document contains.
    pub fn goto(&mut self, index: usize) -> Result<(), EditError> {
        let reachable = self.past.len() + self.future.len();
        if index > reachable {
            return Err(EditError::NoSuchPoint { index, reachable });
        }
        while self.past.len() > index {
            self.undo();
        }
        while self.past.len() < index {
            self.redo();
        }
        Ok(())
    }
}

/// Validate `story` without changing anything, returning every problem found.
///
/// `Story::validate` stops at the first fault, because during editing one fault at a time
/// is what you want to fix. For a pre-save check a complete list is more useful, so this
/// walks the graph itself. Public because the CLI reports the same faults without opening
/// a document, and two implementations of "is this story sound" would drift apart.
#[must_use]
pub fn problems_of(story: &Story) -> Vec<String> {
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
            rustory_vn_core::Exit::End { .. } => {}
            rustory_vn_core::Exit::Goto { to } => {
                push_if_dangling(&mut found, story, id, to, "the goto");
            }
            rustory_vn_core::Exit::Choose { choices, .. } => {
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
            rustory_vn_core::Exit::Gameplay { spec } => {
                if spec.outcomes.is_empty() {
                    found.push(format!(
                            "node `{id}`: gameplay system `{}` maps no outcomes, so the story cannot continue",
                            spec.system
                        ));
                }
                for (outcome, to) in &spec.outcomes {
                    push_if_dangling(&mut found, story, id, to, &format!("outcome `{outcome}`"));
                }
            }
        }
    }

    // Reachability is a content problem, not a structural one: the story loads and
    // plays, but the author has written a scene nothing can arrive at. Reported
    // rather than refused, because a draft branch is a normal state while writing.
    let mut reached = std::collections::BTreeSet::new();
    let mut queue = std::collections::VecDeque::new();
    if story.nodes.contains_key(&story.start) {
        queue.push_back(story.start.clone());
    }
    while let Some(id) = queue.pop_front() {
        if !reached.insert(id.clone()) {
            continue;
        }
        let Some(node) = story.nodes.get(&id) else {
            continue;
        };
        match &node.exit {
            rustory_vn_core::Exit::End { .. } => {}
            rustory_vn_core::Exit::Goto { to } => queue.push_back(to.clone()),
            rustory_vn_core::Exit::Choose { choices, .. } => {
                queue.extend(choices.iter().map(|choice| choice.to.clone()));
            }
            rustory_vn_core::Exit::Gameplay { spec } => {
                queue.extend(spec.outcomes.values().cloned());
            }
        }
    }
    for id in story.nodes.keys() {
        if !reached.contains(id) {
            found.push(format!("node `{id}` cannot be reached from the start"));
        }
    }

    found
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
    from: &rustory_vn_core::NodeId,
    target: &rustory_vn_core::NodeId,
    how: &str,
) {
    if !story.nodes.contains_key(target) {
        found.push(format!(
            "node `{from}`: {how} points at `{target}`, which does not exist"
        ));
    }
}
