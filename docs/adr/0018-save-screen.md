# ADR 0018 — A default save screen, replaceable wholesale

- **Status**: Accepted, implemented
- **Date**: 2026-10-27
- **Open question**: who provides the screen a player saves from?

---

## Context

ADR 0006 settled what a save contains: a snapshot, including a suspended gameplay system's own
state. ADR 0014 gave authors a timeline of their own document states.

A player had exactly one way to save: a file path. `rustory play story.ron --save-slot run1.ron`
worked, and the window wrote that one file on F5. That is enough to verify a feature and not
enough to ship a game, for a reason that is not about the engine: **a player cannot see their
saves.** "Which slot was I on, what did I look like, and when" is a question every commercial
visual novel answers with a screen, and the alternative — remember your file names — is not a
design, it is an absence of one.

So: does the engine provide the screen, or does each game?

## Options

### A. Provide nothing but the file writes

Ren'Py-like engines ship a save screen because their creators asked for one; Godot ships
`FileAccess` and leaves the screen to the game. Both are coherent.

The problem for this project is what "leaving it to the game" actually costs. Every game would
build a grid, a card layout, a hit test, a damaged-file state, an empty state, and a "this save is
from an older version" state — and every one of them would get at least one of those subtly wrong,
because those are the states nobody tests until a player reports them.

### B. Provide a fixed, unchangeable screen

Cheap for the engine, and wrong for the stated purpose. A project with its own menu art would
have to fork the file, which is the definition of the "extension requires editing the core" this
project exists to avoid.

### C. Provide a default, and make it replaceable wholesale

The player holds a `Box<dyn SaveScreen>`. `GridScreen` is what it gets if nobody says otherwise. A
project that wants its own supplies its own and nothing else changes.

## Decision

**C.** Three pieces, each with one job:

- `rustory-vn-core::slots` — the **model**: `SaveSlot`, `SlotLayout`, `SlotSummary`, `SlotIndex`,
  `Thumbnail`. No `std::fs` anywhere, so it is testable in memory like every other core module.
- `apps/rustory-vn/src/slots_fs.rs` — the **I/O**: read, write, delete, timestamps. Atomic writes.
- `apps/rustory-vn/src/save_screen.rs` — the **screen**: a draw list and a hit test.

The screen emits a `GameplayView` rather than draw calls, for the reason ADR 0013 gives: it is
plain data, so layout and hit testing are testable without a GPU, and both front-ends paint it
with code they already have. A save screen that reached for a renderer would be a second drawing
path.

### Drawing and hit testing share one function

`cards(layout)` returns the rectangles. `GridScreen::view` draws them; `GridScreen::slot_at`
tests them. Computing the rectangles twice is how a click lands on the card next to the one it
visibly hit — the same bug ADR 0019's `Viewport` exists to prevent on the pointer side. A test
walks every card's centre and asserts the hit test returns that card.

### Four states, each said in its own words

- **Empty** — "Empty". Clickable in Save, not in Load.
- **Occupied** — title, the line the player was looking at, and the language.
- **Damaged** — "Damaged" plus the parser's reason. Clickable in Save (that is how a player
  recovers from one), not in Load.
- **Wrong version** — Occupied, flagged with why. Still listed, because a player who remembers
  saving wants to know the file is there.

Collapsing any two of these is how a player clicks a slot that does nothing with no idea why. The
model enforces the distinction rather than leaving it to the screen: `SlotSummary::is_empty()` is
**false** for a damaged slot, because it holds something that could not be read.

### The language is on the card

`SaveState` gained a `language` field, and the card shows it. Resuming a save silently switching
language under a player who chose one is the kind of bug that is only noticed by someone who does
not read the new language.

### A pin beats the grid

`--save-slot <path>` still works and still means "this one file". A caller who named a file wants
that file, and a script that resumes one specific save has no use for a grid — so with a pin, F5
and F9 read and write it directly and no screen opens.

## What this does not do

- **No thumbnails yet.** The model carries a `Thumbnail` and the screen emits `Image` elements
  named `save-thumb-<stem>`, but nothing captures one. Capturing means rendering the scene to a
  small texture at save time and keeping the pixels in the save file — a format change and a GPU
  readback, and worth doing once someone has a reason.
- **No delete button on the screen.** `rustory saves <story> --delete <slot>` does it, which is
  enough to get rid of a damaged file. A button needs a confirmation dialog, and that is a
  screen's design decision.
- **No multi-page grid.** Nine numbered slots and one quick slot fit one screen. More slots need
  paging, and the count is already a `SlotLayout` parameter.

## Verification

Model, `crates/rustory-vn-core/src/slots.rs` — 18 tests: zero-padded stems round-trip, a layout's
paths and ordering, a slot past the count is not offered while the quick slot always is, an empty
slot and a damaged one are different things, a summary reads what a menu needs out of a save, the
newest slot skips damaged files, describing all slots follows the layout rather than insertion
order, and a thumbnail whose buffer does not match its size is refused at construction.

I/O, `apps/rustory-vn/src/slots_fs.rs` — 9 tests against a real scratch directory: a written slot
reads back byte-identical, a first save creates its directory, an empty slot reads as nothing, a
corrupt file becomes a damaged slot, a save from another story version is flagged, a slot written
twice holds only the second save and leaves no temporary file, deleting an empty slot is success,
modification times fill the newest slot.

Screen, `apps/rustory-vn/src/save_screen.rs` — 12 tests: nine numbered cards plus a quick slot,
no two cards overlap, every card is inside the design box, the centre of every card hit-tests to
itself, gaps and outside points hit nothing, a narrow layout still fits, empty and damaged cards
say so, an occupied card shows title/line/language, the two modes are distinguishable, and a
thumbnail produces an `Image` element named after its slot while a slot without one produces none.

End to end with the real binary:

```console
$ rustory save stories/demo.ron --advance 5 --out stories/saves/slot00.ron
wrote stories/saves/slot00.ron at node `honest` (revealing)
$ rustory saves stories/demo.ron
stories\saves
1: RuStory demo at `honest`
2: empty
...
$ rustory saves stories/demo.ron --json
{"dir":"stories\\saves","newest":"1","occupied":1,"ok":true,...}
$ rustory saves stories/demo.ron --delete 1
deleted slot 1
```

`rustory save` also learned to create its `--out` directory: `--out stories/saves/slot00.ron` on a
fresh project used to fail with "no such path", which is a bad first five minutes for a command
the README documents.
