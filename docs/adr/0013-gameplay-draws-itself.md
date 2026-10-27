# ADR 0013 — A gameplay system draws its own screen

- **Status**: Accepted, implemented
- **Date**: 2026-10-27

---

## Context

Until now a gameplay system could say exactly one thing to the player: one string, which the
engine drew in a band where the dialogue box would have been. `TimedChoice` used it to print
`"Choose quickly   (2.4s)\n1. …    2. …"`. `Quiz` printed a header, a question and its options
on three lines.

Both worked, and both were already lying about their own state. The countdown had a number
where a shape belonged; the quiz had a list where a row belonged. `architecture.md` recorded the
limit plainly: *a board, a highlighted cursor or a drag target does not fit*.

That is a problem for a project whose stated purpose is that a visual novel can host **any**
kind of gameplay. A string is enough for a dialogue and not enough for a puzzle. If the only
way to build a board is to encode it as text and hope the player reads carefully, then the
engine has a preferred genre, and it is not the one the project promises.

## Options

### A. Keep the string; let systems format harder

Zero engine cost. Also a dead end: alignment, emphasis and shape cannot be recovered from a
string, and every system would invent its own ASCII layout. A `Bar` drawn out of `[####----]`
is the kind of thing that looks clever in a config file and cheap on screen.

### B. Give a system the draw list

`Vec<Quad>` and `Vec<TextItem>` — the renderer's own types. Maximum power, and it collapses the
boundary this project is built on: `rustory-vn-core` would either depend on the renderer or
duplicate its types, and every gameplay author would become responsible for typography, wrap
widths and font sizes. The result is not "any gameplay", it is "any gameplay that also ships a
UI implementation".

There is a second cost, less obvious and worse: a system that places its own pixels also
decides its own look. Move it to another project and it arrives with a visual identity that
does not match.

### C. A small set of semantic elements, in the engine's design box, coloured by role

A `GameplayView` is a list of four kinds of thing — text, panel, image, bar — positioned in the
same 1280×720 box the engine's own layout uses, coloured by role (`Tone`) rather than by value.

Enough for a board (panels and images), a cursor (a panel), a health bar (a `Bar`), a scoreboard
(text). Not enough to place a raw pixel, load a font or name a colour — and those three
omissions are what keep a system portable and the palette the project's.

## Decision

**C.**

```rust
pub enum Element {
    Text  { text: String, rect: ViewRect, size: f32, tone: Tone, align: Align },
    Panel { rect: ViewRect, tone: Tone, border: Option<Tone> },
    Image { name: String, rect: ViewRect },
    Bar   { rect: ViewRect, fraction: f32, tone: Tone },
}
```

`GameplaySystem::prompt` is **removed**, not deprecated. Every system returns a view or nothing;
there is no second path, so no system can quietly stay on the old one.

### The palette is the project's, and lives in the renderer

`Tone` names a role. `Palette` maps roles to `Rgba`, with defaults that match the engine's dark
scene, and a project overrides the fields it cares about. It lives in `rustory-vn-render`, next
to the colours it produces, rather than in a story file — a palette is presentation, and storing
it in the script would make two projects' files incompatible for no reason.

This is also why `paint_gameplay_view` lives in the renderer. There are two front-ends already
(the player and the editor's preview) and the editor has historically duplicated the *story*
layout, with a comment explaining why: hoisting story concepts into the renderer would be wrong.
A view is not a story concept — it is rectangles and runs of text — so it can be shared, and
sharing it is what stops the preview from drifting away from what ships. `rustory-vn-render`
gains a dependency on `rustory-vn-core`, which is the allowed direction.

### Coordinates are the engine's box, not the window's

1280×720, the same numbers `scene.rs` lays dialogue out in. A system never learns the window
size. The window letterboxes the box (`letterbox` in `play.rs`), so resizing cannot change the
rules and a headless `shot` renders the same frame the player sees.

Normalised `0..1` coordinates were considered and rejected: they read as device-independent and
are harder to write (a 64-pixel row is `0.05` of the height), and every existing layout constant
in the engine is already in this space.

### Four elements is a promise, not a starting point

Each element is code the engine must keep drawing for as long as it exists, and each one is a
place where a system can look wrong in someone else's project. Four covers the shipped systems
and the guide's example. A fifth is added when a system cannot work without it, and the request
is the review — not when it seems like it might be useful later.

### An opaque panel is opaque on purpose

`Tone::Panel` defaults to alpha 255. Blending happens in linear light, so a nominally near-opaque
252 lets about 8% of a sprite through and a face reads through a title. Measured in
`shots/gameplay-timer.png`: (200,300) sampled 27,27,31 with alpha 252 and 6,8,12 with 255.

A system that wants the story to show behind it uses `Tone::Raised` on a smaller rectangle. That
is a decision a system makes deliberately; leaking sprites through a full-screen backdrop is not.

## What this does not do

- **Pointer input.** Input is still `Confirm` and `Select { index }`. A drag needs coordinates,
  and inventing that API before a system that drags exists would be guessing. The view side is
  ready for it; the input side is the next ADR.
- **Text alignment beyond left and centre.** Justified text in a dialogue box looks like a
  mistake, and a system that needs right-aligned numbers has not appeared yet.
- **Clipping.** A view cannot say "cut this off at the panel edge". A system that draws outside
  its rectangle is drawing outside its rectangle.

## Consequences

- The countdown is a bar, not a number in parentheses; the quiz is a column of rows with a
  progress bar, not a `\n`-joined string.
- `TimedChoice` and `Quiz` both changed, and the guide's example changed with them. That is the
  cost of removing `prompt` instead of leaving both paths alive.
- `Element::Image` needs a name the project's texture bank knows. Missing art is skipped, not
  boxed — the same rule sprites follow.
- `rustory-vn-render` now depends on `rustory-vn-core`. It did not before. The direction is the
  permitted one (ADR 0001), and the alternative — a third crate whose only job is one function —
  was not worth a crate.
- `rustory shot` gained `--at <node>:exit`, because otherwise there is no way to screenshot a
  gameplay screen: clicking through in a CLI would play the whole interaction to completion.

## Verification

- `crates/rustory-vn-core/src/view.rs` — 7 tests: element order, a clamped fraction, NaN
  handling, empty views, visible-area checks.
- `crates/rustory-vn-render/src/view.rs` — 8 tests on the painter: quads per element kind, fill
  width follows the fraction, alignment and colour survive, missing art draws nothing.
- `crates/rustory-vn-gameplay-timer` — the view test asserts the options appear as text *and*
  that the bar's fraction drops from 1.0 to 0.75 after a quarter of the countdown.
- `crates/rustory-vn-gameplay-quiz` — asserts the round counter, the question, the answered
  list and a progress bar at one third.
- `apps/rustory-vn/src/scene.rs` — a full-screen panel covers the stage, a half-filled bar fills
  half its track, a zero-sized element draws nothing, a gameplay image resolves through the
  texture bank.
- Rendered and inspected: `shots/gameplay-timer.png`, `shots/gameplay-quiz.png`, produced by
  `rustory shot --at honest:exit` and `--at confession:exit`.
- Both front-ends were given the same story and the same interaction count
  (`rustory shot --advance 3` and `rustory-editor --check --step 3`) and produced PNGs with
  the same SHA-256. That is what "shared painter" means in practice, not just in intent.
