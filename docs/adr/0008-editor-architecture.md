# ADR 0008 — The editor is an operation layer with two front-ends

- **Status**: Accepted, implemented
- **Date**: 2026-10-27

---

## Context

The project goal is an editor that is comfortable for a person and usable by an agent,
with a GUI for humans and a CLI for agents. The naive reading is "build a GUI, then add
a CLI that does the same things" — two programs that agree because someone keeps them
agreeing.

That is how editors rot. The GUI gets a feature on Tuesday; the CLI gets it in a
quarter, or never, or differently. An agent then cannot do what a person can, and the
promise quietly becomes false.

## Options

### A. GUI first, CLI as a wrapper around the GUI

Impossible to do well: driving a UI from a script means synthesising clicks, and the
result is as reliable as screen coordinates.

### B. Two independent implementations

Both are real, both are maintained, and they drift. Every rule about the format has two
copies, one of which is stale.

### C. One operation layer, two front-ends

A crate defines the document, the operations, and the guarantees. The GUI and the CLI
both construct operations and hand them to the same executor.

## Decision

**Adopt C.** [`rustory-edit`](../../crates/rustory-edit) is that layer.

```
        rustory-edit
        ├── Document   story + path
        ├── EditOp     every change, one enum
        └── Editor     apply · validate · rollback · undo · save
                ▲
      ┌─────────┴─────────┐
 rustory-editor        rustory (CLI)
   (egui GUI)          (ops · inspect · edit)
```

## The guarantee that makes agents safe

`Editor::apply` returns `Ok` with the change in place, or `Err` with the document
**exactly** as it was. No partial edits, ever. This is implemented by snapshotting
before the operation and restoring on any failure — including a failure of the story's
own validation.

A human sees the screen change and reacts. An agent needs "it failed" to mean nothing
happened, or it will build on a state that does not exist.

The same reasoning produced two refusals rather than conveniences:

- **Deleting a referenced node is refused**, naming every referrer, instead of
  repointing references automatically. Guessing where the author meant them to go is
  worse than saying it cannot be done.
- **Deleting the last choice is refused** instead of silently converting the exit to
  `End`. That would change what the story does without saying so.

## Why egui

- It is immediate-mode, so the editor's state *is* the UI's state. No widget tree to keep
  in sync with the document, which is the class of bug that makes editors feel unreliable.
- `egui-wgpu` 0.37 takes `wgpu ^30`, the exact version `rustory-render` uses. Sharing one
  device means the preview can render with the engine itself.
- No C++ toolchain, no bindings, no second build system.

The rejected alternative was a web front-end (Tauri). It would give better typography and
a real layout engine, but it also puts a second language and a browser engine between the
editor and the document — for a tool whose value is that the operations are explicit.

## The preview is the engine, not a mock-up

`rustory-editor` builds the same `Frame` that `rustory play` renders, using
`rustory_render::Renderer`. A preview that approximated the output would be worse than
none, because it would be trusted.

The preview starts at the node selected in the node list, not at `story.start`. A preview
of one node while another is selected is a panel that lies about what it is showing. The
header text is taken from the same `Narrative` that produced the frame, for the same
reason: two answers to "where am I" eventually disagree.

It is also playable. The position is a path of steps (`Click`, `Choose(i)`) rather than a
count, so clicking the frame advances, branches appear as buttons, and the slider rewinds
along the path already taken. Replaying a path against an edited story lands where the
clicks did; a branch that no longer exists falls back to advancing rather than wedging
the preview.

Layout code is duplicated between `apps/rustory/src/scene.rs` and
`apps/rustory-editor/src/preview.rs`. That duplication is a **known cost**, recorded here
rather than hidden: the alternative is hoisting layout into `rustory-render`, which would
put story concepts into the renderer and break invariant 5 of the architecture. The
mitigation is that both call the same `rustory_render` primitives, so an engine change
shows up in both, and the duplication is layout constants only — no game logic.

This is the one place the architecture has a seam the compiler does not check. If a third
front-end appears, layout should move into `rustory-edit` as a "scene description"
intermediate, and this ADR should be superseded.

## What the GUI deliberately does not do

- **No feature that changes the story without an `EditOp`.** If a feature is not
  expressible as an operation, it belongs in the operations first. Selection, the recent
  list and the preview position are not edits — see "Selection is UI state" below.
- **No playback controls.** The preview steps; it is not a player. `rustory play` is the
  player, and giving the editor a second input loop would mean two answers to what a
  click does.

## Verification

- `cargo test --workspace` — 245 tests. `rustory-edit` contributes 65 of them, covering
  every operation, every refusal, batch rollback, undo/redo, and a lossless save/reload
  round-trip including non-ASCII text.
- `rustory ops --json` prints the operation vocabulary with `changes_structure` flags.
- `rustory inspect --json` prints the story as data with the same schema on success and
  failure.
- `rustory edit` refuses a batch atomically; `--dry-run` proves the file is untouched.
- `rustory-editor --check <story>` renders one preview frame without a window and fails
  if the result looks blank — the regression test a GUI otherwise cannot have.
- The screenshots in `shots/editor.png` were produced by running the real binary.

## Known gaps

- **No recent-files list.** Reopening means going through the dialog again.
- **The preview does not re-render on window resize.** The logical resolution is fixed,
  so this only shows up as letterboxing differences.
- **No colour grading or theme editor.** Layout constants are compiled in; a project that
  wants a different dialogue box position edits `apps/rustory-editor/src/preview.rs` and
  `apps/rustory/src/scene.rs`.
- **Native-texture display was attempted and abandoned.** The texture the engine drew
  into and the one egui sampled were not the same object, so the panel showed a stale
  blank frame; a magenta clear colour proved it, because a colour that cannot survive any
  conversion failing to appear meant the displayed texture was not the drawn one. The
  readback path is slower per edit and reliable. Recorded so nobody re-attempts it without
  new information.
- **A stale `eframe` build cache produced a `windows_sys` version conflict** after `rfd`
  was added. `cargo clean -p eframe -p egui -p rfd` resolved it; nothing was wrong with the
  dependency graph itself. Worth knowing because the error message (`found possibly newer
  version of crate windows_sys`) points at the graph rather than the cache.

## What was added after the first version

Both gaps this ADR originally listed are closed:

- **File dialogs.** New / Open / Save / Save-as, via `rfd`, isolated in `dialogs.rs` so the
  blocking behaviour is confined and the dependency is replaceable in one file. A new
  story starts untitled and `save` refuses to invent a path — writing to a guess is how
  work ends up somewhere the author never looks.
- **Gameplay outcome mappings.** `SetOutcome` and `DeleteOutcome` joined the operation
  layer, which means the GUI gained the ability *because the CLI did*: adding the
  operations first is what kept the two front-ends from diverging.

A close guard was added at the same time. Unsaved work prompts before the window closes
and before Open or New replaces the document, because losing a scene to a misclick is the
one failure an editor must not have.

## Recent files

The recent list answers "which story was I editing?" with one click, and it is kept in
the app rather than in the operation layer. Which files a person opened last week is a
property of the workstation, not of the document: it must not become an `EditOp`, it must
not enter the undo history, and it must not be written into the story file where it would
follow the project to another machine and mean nothing there.

- **Storage.** `%APPDATA%\rustory\recent.json`, `$XDG_CONFIG_HOME/rustory/recent.json`
  (or `~/.config`) elsewhere, ten entries at most. Stored absolute, because a list
  written from one working directory has to resolve from another. Compared
  case-insensitively on Windows, where `Story.RON` and `story.ron` are one file.
- **Tolerance over strictness.** A missing or corrupt file loads as an empty list, and an
  entry that no longer opens is dropped: every item in the menu has to be a file that
  opens, so the list is pruned on startup rather than offering a row of errors.
- **The CLI reads, the GUI writes.** `rustory-editor --recent [--json]` prints the list
  for an agent to inspect without a display. It deliberately does not prune, because a
  terminal command that silently rewrites the user's configuration is a surprise; the
  editor will drop the dead entry the next time it runs.

## Selection is UI state

`selected` lives in the app, not in the document, and panels set it directly. Choosing
what to look at is not editing: it takes no undo step, changes nothing on disk, and is
not an `EditOp`. The rule "panels describe intent, the app applies it" is about edits.

That is what makes navigation cheap enough to put everywhere. The **Go** button beside a
`goto` target, a choice target and a gameplay outcome all write that one field, so
following a branch does not mean remembering a node name and scrolling to find it. The
node list has a filter for the same reason �� a story with thirty nodes is a list you scan,
and scanning is what a filter is for. Neither feature touches the story, which is why
neither needs an operation, a test of atomicity, or a place in the CLI.

## Recovering work, and the cost of rewriting a file

Two problems that only appear after people have been using the editor for a while.

**A crash loses everything since the last save.** The close guard covers polite exits and
nothing else — a power cut, a driver crash, a closed laptop lid do not ask. While a document
has unsaved changes it is therefore written beside the story as `<story>.draft.ron`, two
seconds after typing stops. On the next open the author is asked whether to take it, and
"no" deletes it: both answers are destructive, so neither can be the silent default. A draft
that will not parse is discarded without asking — the file on disk is still there, and
offering a document that will not open turns a recovery into a puzzle.

**Saving deletes comments.** A story is a data structure; RON comments are not part of one.
Saving rewrites the file from the structure, so notes an author wrote in their script are
gone, and the formatting is the editor's. This is inherent to an editor built on
`EditOp`s over a typed model (ADR 0008's premise), and preserving comments would need a
span-preserving parser — a much larger piece of software than the editor itself.

The choice was to be honest rather than silent: `Document::source_has_comments` detects the
case roughly (a `//` inside a string counts, which is the safe direction to be wrong in) and
the editor warns once per document before the first overwrite. A person who keeps notes in
their script can decline the save and use the CLI, which writes the same structure but is
used deliberately rather than by muscle memory.