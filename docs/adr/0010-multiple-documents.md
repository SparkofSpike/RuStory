# ADR 0010 — Several documents, one window

- **Status**: Accepted, implemented
- **Date**: 2026-10-27

---

## Context

ADR 0008 listed "one document at a time" as a known gap: comparing two chapters meant two
windows. A visual novel is rarely one file — a project is a prologue, a handful of branches
and an ending, and the work alternates between them.

The editor's state had grown flat: `editor`, `selected`, `node_filter`, `preview_steps`,
`draft`, `config_buffer`, and so on, all fields on one `EditorApp`. Adding a second document
meant either duplicating that list or deciding, field by field, which ones are about a story
and which are about the window. The second is the only answer that stays correct, and it is
the whole of this ADR.

The reference project (GEO) supplied the failure mode to avoid. Its concurrent article
streams shared one `AbortController` and one idle timer; when a stream finished it cleared the
other's timer and nulled its controller, and the second stream then threw from inside a timer
that no longer belonged to it. The fix was to give every task its own handles. The lesson
generalises: **any mutable thing that belongs to a document must live on the document.**

## Options

### A. Clone the state list per tab

`Vec<Editor>` plus parallel `Vec<Option<NodeId>>`, `Vec<Vec<Step>>`, `Vec<Option<Draft>>`, …

Index-parallel vectors do not stay in step. One early return that pushes to three of them and
not the fourth is a bug nobody finds until a rename. The compiler cannot tell two `Vec`s of
equal length are the same list.

### B. A `Tab` type, and an app that holds several

Everything about one story lives in `Tab`. Everything about the window — the renderer, the
status line, the recent list, the help window — stays on `EditorApp`.

Rejected variant: a `Tab` that owns its own `Preview`. Each `Preview` holds a `Renderer`, which
holds a wgpu device. One GPU device per open story is not a feature.

### C. Separate windows (a second `EventLoop` per document)

Real multi-window, real complexity: shared undo across windows, a recent list per window, and
an `eframe` instance per document. For a tool whose whole point is that a document is cheap,
this is the expensive answer.

## Decision

**B.** `Tab` in `apps/rustory-vn-editor/src/tab.rs`; `EditorApp { tabs: Vec<Tab>, active: usize }`.

The split follows one question: *can this field name a node or a file?* If yes it is on the
tab. The interesting cases:

- **`preview_steps`** — a position in *this* story. On the tab.
- **`assets`** — art beside *this* file. On the tab, derived from its own path.
- **`draft` / `draft_due`** — the recovery copy of *this* document, and when to write it. On
  the tab. A shared deadline would write tab A's story into tab B's `.draft.ron`, which is a
  data-loss bug that would be blamed on the filesystem.
- **`config_key` / `config_buffer`** — form state for *this* node in *this* story. On the tab.
- **`pending: Vec<EditOp>`** — an operation names a node in this story; applying it to
  whichever tab happened to be active at flush time is how edit B's story with A's intent.
  On the tab, and `flush` empties *every* tab's queue, not just the visible one.
- **`preview`** — the GPU renderer. On the app. Shared.
- **`status`** — a line about the last thing that happened in the window. On the app.

## The rules that follow

**Opening a file already open switches to it.** Two tabs on one path would each hold a copy,
and the last save would silently win. `find_tab` compares case-insensitively on Windows and
normalises separators, without `canonicalize` — the file may not exist yet, and canonicalising
on Windows adds a `\\?\` prefix that breaks the comparison for exactly the case it exists to
catch.

**Closing a tab asks about that tab.** `close_tab` → `confirm_discard(index)` → `remove_tab`.
The split is deliberate: the index arithmetic is testable without a dialog, and it is where an
off-by-one hides.

**Closing the window asks about every dirty tab, in order.** All of them, not the visible one.
A window with three unsaved chapters and a guard that looked at one would lose the other two
without a word.

**Shortcuts act on the active tab**, and `Ctrl+W` closes it.

**An empty tab list is legal.** Closing the last tab leaves a window that offers to open a
story, which is better than a window that refuses to close.

## The borrow pattern this forces

Panels hold `&mut app.tabs[i]` and write fields (`tab.pending`, `tab.selected`). They do not
call methods on `Tab` while holding a borrow of `tab.editor`, and they do not call
`app.set_status` from inside a closure that borrowed a tab. Both would take the whole struct.

One consequence is visible in the code: `Tab::plan_selection` is an associated function taking
`&mut Option<NodeId>` and `&mut Vec<Step>` rather than a `&mut self` method. That is not a
style choice — a method cannot be called from any of the three places that set a selection,
because each of them is already borrowing the editor at the same time.

## Verification

- `apps/rustory-vn-editor/src/tab.rs` tests: a new tab selects the start node; an untitled tab has
  no draft; **two tabs write two distinct drafts with the right content in each**; a write
  before the debounce writes nothing; saving and discarding both remove the draft; selecting a
  different node resets the preview path and reselecting the same one does not; a stale
  selection falls back to a node that exists.
- `apps/rustory-vn-editor/src/app.rs` tests: closing a tab to the left keeps the same document
  active; closing the active tab lands on the one that slid into its place; closing the last
  tab pulls the index back; closing the only tab leaves an empty window; an out-of-range close
  does nothing; opening the same path twice yields one tab; path comparison ignores case and
  separators on Windows.
- Observed in the running window: `New` produced a second tab labelled `untitled`, the panels
  switched to that story's one node, and clicking the first tab restored `demo.ron`'s node
  list, selection, preview frame and status line.

Not verified by screenshot: the "unsaved changes" dialog on closing a dirty tab. The decision
logic is unit-tested and the dialog is the same `rfd::MessageDialog` path that ADR 0008's close
guard already used, but mouse input to some widgets proved unreliable in this environment and
the honest report is that the click was not observed to land.

## Consequences

- The renderer is shared, so a tab switch costs one re-render, not a new device.
- The preview cache key is `(story checksum, start node, steps)`. Two tabs showing identical
  stories share a frame, which is correct; the key does not include a tab id on purpose.
- Undo history stays per document, which is what an author expects: `Ctrl+Z` in chapter two
  must not undo something in chapter one.
- A project-level view (a tab per chapter in a sidebar, a rename-across-files operation) is not
  this. It needs a different model, and should get its own ADR before any code.
