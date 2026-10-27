# ADR 0004 — Cross-system contract: events outward, narrow state inward

- **Status**: Accepted, implemented
- **Date**: 2026-10-26
- **Open question**: how do narrative and gameplay share state?

---

## Context

The narrative and a gameplay system have to exchange information, or "gameplay
takes over the narrative" is not implementable at all. The question is how.

The original constraint was "separate data from behaviour; couple through events
and a small amount of shared data". That left three things unspecified: how small
"small" is, what an event looks like, and who may read what.

## The three channels

```
                 ┌──────────────────┐
   script        │    Narrative     │  NarrativeEvent ──▶ renderer / log / save
   outcomes ────▶│  (state machine) │
                 └────────┬─────────┘
                          │ GameplayContext { dt, inputs, &mut Stats }
                          ▼
                 ┌──────────────────┐
                 │ GameplaySystem   │ ──▶ OutcomeKey (the only channel back)
                 └──────────────────┘
```

### 1. Narrative → outside: the event stream

`NarrativeEvent` is read-only, enumerable and `Clone`. The renderer, logging and a
future save system subscribe; none of them mutate state in return. **This direction
is one-way**, and that is why the narrative state machine stays reason-able.

### 2. Narrative → gameplay: `GameplayContext`

Everything a system can reach:

```rust
pub struct GameplayContext<'a> {
    pub dt_ms: u32,                  // how long this frame took
    pub inputs: &'a [GameplayInput], // input intents, already translated
    pub stats: &'a mut Stats,        // the only shared writable state
}
```

**Not reachable**: the stage (background, sprites), the story graph, the renderer,
the window. Not by convention — by type. None of those exist in the struct.

### 3. Gameplay → narrative: `OutcomeKey`

One channel back, and it is a string key. The core looks it up in the script's
`outcomes` table.

## The shape of `Stats`

```rust
pub struct Stats { values: BTreeMap<String, i32> }
```

- Keys are strings: different stories and different systems need different
  attribute names (affection, stamina, suspicion).
- Values are `i32`: a fixed type.
- **Not** `BTreeMap<String, Box<dyn Any>>`. This is the line between this project
  and "dynamically shaped improvisation". Attribute names may be free; value
  semantics must be uniform.

## Decision

Adopt the three-part arrangement above: **events outward, a narrow context inward,
a single key back.**

## Reasoning

1. **Reason-able.** Every change to narrative state can only originate in three
   places: `tick`, `advance`/`choose`, and `resolve_gameplay`. No gameplay system
   can mutate narrative state behind the core's back.
2. **Testable.** Everything in `GameplayContext` is a plain value, so testing a
   system needs no story. All nine tests in `rustory-vn-gameplay-timer` run purely in
   memory.
3. **Expansion is explicit.** If a system genuinely needs to read the stage — "judge
   the click by sprite position", say — that cannot be solved by adding a field.
   It requires an ADR arguing why that is not a boundary violation. The friction is
   intentional.

## Consequences

**Gained**

- Gameplay is fully decoupled from presentation. The same timed choice runs in a
  window, in a headless screenshot pipeline, and in a test.
- The event stream lets the renderer stay stateless: re-read `Narrative` each frame
  instead of maintaining an incremental mirror.

**Paid**

- State shared between systems must go through `Stats` string keys, losing
  compile-time spelling checks. Mitigation: define key names in one place, with
  namespaced constants such as `timed_choice.attempts` and
  `timed_choice.correct`.
- The event queue relies on the caller invoking `take_events()`. **No production
  code calls it today** — the renderer re-reads state each frame and has no use for
  events, and neither the CLI nor the windowed player subscribes. The queue is
  therefore self-bounded (`MAX_PENDING_EVENTS = 256`, oldest dropped first);
  without that, a long session would leak memory in proportion to beats played.
  If a real consumer appears (sound cues, achievements, a log), it should drain
  every frame, and the bound will never be reached.

**Known gap (recorded as an open question)**

- There is no channel for gameplay to **read** narrative state. A "debate battle"
  that wants to know which dialogue round it is in can only be told through config.
  Supporting this should mean adding a **read-only** `NarrativeSnapshot` (node id
  plus beats played plus `Stats`) rather than exposing `Narrative` itself. That
  change needs an ADR.
