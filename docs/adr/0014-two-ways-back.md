# ADR 0014 — Two ways back: save slots for players, a timeline for authors

- **Status**: Accepted; the timeline is implemented, save slots are file paths (see below)
- **Date**: 2026-10-27
- **Open question**: how does someone return to an earlier state?

---

## Context

ADR 0006 settled *what* a save contains: a snapshot, not an event log, including a suspended
gameplay system's own opaque state. It did not settle who gets to make one, or how many.

Two people use this engine and their needs only look alike:

- **A player** wants to stop playing and resume later, and to keep more than one stopping
  point — "before the boss", "the ending I liked". The state that must survive is a
  `SaveState`: cursor, beat, stage, stats, transcripts, gameplay internals.
- **An author** wants to get back to a version of their own work: the scene before the rewrite,
  the branch that still made sense. The state that must survive is a `Story`, plus enough
  history to move between versions.

The editor has kept every intermediate `Story` since its first commit — undo is a stack of
whole stories — but the states were reachable only by pressing Ctrl+Z the right number of
times. A stack you cannot see is a stack you do not trust.

## Options

### A. Leave it: undo/redo shortcuts, one save file

Cheap, and what the engine had. It breaks down exactly when a session is long: nobody counts
undo steps, and a creator who has just lost an hour's work to a wrong branch does not care
that the state exists somewhere in a 128-entry stack.

### B. One unified "checkpoint" abstraction over both

Tempting symmetry, and wrong. The two states have different shapes — `SaveState` refers to a
story that is already loaded; a timeline point *is* a story. To hold both, a checkpoint would
need a type-erased payload, which is the `Box<dyn Any>` shape this project refuses on principle.
The shared word "save" hides a real difference; sharing a type would not make the difference go
away, only make it unchecked.

### C. Two mechanisms, one idea: a visible list of states you can return to

- **Players**: a save is a file, and a file is a slot. The CLI already takes a path
  (`rustory save … --out run1.ron`, `resume … run1.ron`), so slots exist without any new
  concept; the window keeps one named slot per story (`F5` / `F9`).
- **Authors**: the editor exposes the stack it already has. `Editor::timeline()` lists every
  reachable state with the label of the operation that produced it; `Editor::goto(index)` jumps
  there by repeating undo or redo. The GUI shows it as a panel on `Ctrl+H`.

## Decision

**C**, with the author side implemented now.

`goto` is deliberately not a second history mechanism. It walks the same `past` / `future`
vectors the keyboard shortcuts walk, which is what makes it impossible for the panel and
Ctrl+Z to disagree about the document. The alternative — storing named snapshots beside the
stack — would be a second source of truth, and the first bug would be a panel listing a state
the document can no longer reproduce.

The panel shows redo states as well. An author who has stepped back five times is usually
looking for one of the five they just left; hiding them makes the list feel like it lost work.

## What this does not do

- **Persist the timeline across sessions.** History lives in memory. Writing it to disk means
  deciding where (beside the story? a project directory?), how much (128 stories is fine in
  RAM, less so on disk), and how it interacts with git — none of which a user has asked for
  yet. A named checkpoint file is the obvious next step, and it is a `Save as` away.
- **Give the window multiple named save slots.** The path-based slots are reachable from the
  CLI and from tests; the window has one. A slot picker needs a screen the game does not have
  yet (no title menu, no pause menu), and inventing that screen belongs to the game, not to a
  save-system ADR.
- **Roll back gameplay.** ADR 0006 already says why: the snapshot exists, the ring of past
  snapshots does not, and choosing a point needs UI.

## Consequences

- The history panel is the first editor feature that exists because of an author's workflow
  rather than an agent's API. It still goes through `Editor`, so the CLI and the panel share
  one implementation.
- `EditError::NoSuchPoint` is a new stable error code (`no_such_point`), so an agent that jumps
  too far gets a branchable answer rather than a sentence.
- A jump marks the document dirty, as undo does. An author who jumps back to the opening state
  and saves has genuinely written a different file, and pretending otherwise would be a lie
  about the disk.

## Verification

- `crates/rustory-vn-edit/tests/timeline.rs` — 8 tests: an unedited editor has one point;
  each operation adds one and the last is current; a backward jump rewinds the document and
  keeps the future; a forward jump reapplies it; jumping to the current point changes nothing;
  a jump past the end is refused with `no_such_point` and leaves the document alone; a new edit
  after a jump truncates the timeline; and a 20-edit session agrees with plain undo/redo at
  every one of six stops.
- `apps/rustory-vn-editor` compiles with the panel wired to `Ctrl+H`, the toolbar button and the
  help sheet; the panel's behaviour is `Editor`'s, which the tests above cover.
