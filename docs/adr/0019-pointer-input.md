# ADR 0019 — Pointer input, two phases and no hover

- **Status**: Accepted, implemented
- **Date**: 2026-10-27
- **Open question**: how does a player touch a board the engine let a system draw?

---

## Context

ADR 0013 gave a gameplay system a screen: text, panels, images and bars in a 1280×720 design box.
It left the other half undone, and said so:

> A system receives `Confirm` and `Select { index }` and nothing else. A countdown and a quiz fit;
> a drag, a board cursor or a click on a thing rather than a numbered option does not.

So a system could *draw* a board and the player could not touch it. That made "the VN can host any
kind of gameplay" true for turn-based menus and false for everything spatial — which is most of
what the sentence is for.

## Options

### A. Position on `Confirm`

```rust
Confirm { x: f32, y: f32 }
```

One variant, no new concept. Also breaks every system that matches `GameplayInput::Confirm`: the
countdown and the quiz would both have to be rewritten, and a guest that greps for `Confirm` would
start seeing a struct where it expected a unit variant. Two systems did not need a position, and a
widen-the-existing-thing change makes the systems that do not care pay for the ones that do.

### B. A separate `Pointer` variant, two phases

```rust
Pointer { x: f32, y: f32, phase: PointerPhase }  // Pressed | Released
```

Additive: existing systems match `Confirm` and ignore everything else. A press and a release are
what a button, a drag and a board placement all need.

### C. Add `Hover` and `Moved` as well

A complete pointer model. Also a per-frame event stream at a much higher rate than anything else
in this contract, which every system would have to handle or ignore deliberately. No shipped or
planned system needs it: a clicker wants press, a dragger wants press-then-release-at-a-new-point,
and a board cursor can be drawn from the last press.

## Decision

**B.** `GameplayInput::Pointer { x, y, phase }` with `PointerPhase::{Pressed, Released}`, in the
same design box the view is laid out in.

### Design-box coordinates, not window coordinates

The same reason the view is in that box: a resized window must not change the rules. Compliance is
the app's job, and it is one function:

```rust
Viewport::fit(width, height).to_design(px, py)
```

`Viewport` is also what `letterbox` uses to scale the other way, so the two directions cannot
disagree — which is the bug where a click lands just beside the button it visibly hit. Four tests
pin it, including that a click in a letterbox bar clamps into the box and that a minimised window
(zero scale) produces finite coordinates rather than `NaN`.

A `NaN` coordinate is worse than a wrong one: every rectangle test against it is false, so a
system silently stops responding forever.

### The app sends both `Pointer` and `Confirm`

On a press inside a handover, `play.rs` pushes `Pointer { …, Pressed }` and `Confirm`. That is
deliberate:

- A countdown that has always treated any input as the answer keeps working unchanged.
- A board that only looks at `Pointer` cannot be answered twice, because it ignores `Confirm`.
- A system that wants both decides, and the decision is visible in its own `step`.

`Released` is sent only inside a handover. Outside one, a click is complete when the button goes
down — waiting for the release would make the story feel unresponsive, and there is no drag to
support there.

### No `Hover`

Adding it later is additive; adding it now would be an untested third phase every system has to
switch on. The trigger to revisit: a system whose behaviour depends on where the pointer is *while*
it is not pressed — a tooltip, a cursor that follows the mouse across a board.

## What this does not do

- **No drag gesture.** A system implements one by remembering the press and deciding on the
  release, which is exactly the two phases provided. A `Drag` variant would be sugar.
- **No multi-touch.** One pointer, one position. A two-finger gesture would need identity, and
  inventing it before a system asks is the speculative interface this project refuses.
- **No pointer in the editor's preview.** The preview still uses synthetic input; making it
  playable is a separate piece of work, and ADR 0018's storage layer comes first.

## Consequences

- `GameplayInput` gains a variant. It is `#[non_exhaustive]` nowhere, so a third-party `match`
  without a wildcard stops compiling — which is the right failure for a contract change: loud, at
  build time, naming the file.
- `GameplayInput` loses `Eq`, because `f32` has none. `PartialEq` remains.
- The wasm boundary needed no change: `GameplayInput` was already serialised as RON, and the new
  variant serialises as `Pointer(x:420.5,y:370.25,phase:Pressed)`. A guest written in another
  language greps for `Pressed` and two numbers, which `examples/wasm-guest` now does.
- `play.rs` tracks the pointer position between events. `None` until it has been inside the window
  once, because a default `(0, 0)` is a real position — the top-left corner — and a system would
  act on something the player never pointed at.

## Verification

- `crates/rustory-vn-core/tests/pointer_input.rs` — 6 tests: a press inside a card picks it, a
  press on the background picks nothing, a release is not a press, `Pointer` and `Confirm` together
  do not double-answer, the wire format keeps the variant name and coordinates, and a full story
  run where a press on the right card takes the branch that card maps to.
- `apps/rustory-vn/src/play.rs` — 4 new tests on `Viewport`: a click maps back to the rectangle it
  hit, a click in the letterbox bar clamps into the box, the identity viewport is the identity, and
  a zero-sized window produces finite coordinates.
- `examples/wasm-guest` now answers a press: the 1.6 KB module hit-tests the two door cards it
  draws and reports `won` or `lost`, so the pointer path is exercised through the sandbox as well
  as through a Rust system.
- End to end: `rustory check stories/doors.ron` → `plugin doors: plugins\doors.wasm (directory)`,
  `rustory transcript stories/doors.ron` → the guest decides, the story branches.
