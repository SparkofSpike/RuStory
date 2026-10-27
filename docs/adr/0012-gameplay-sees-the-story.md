# ADR 0012 — A gameplay system can see where the story is

- **Status**: Accepted, implemented
- **Date**: 2026-10-27

---

## Context

ADR 0004 said a gameplay system may touch `Stats` and its own fields, and nothing else. That
was right for the systems that existed: a countdown and a quiz both decide everything from their
config.

It was not right in general, and `architecture.md` recorded the gap: *"A system cannot tell
which dialogue round it is in; it has to be passed that through config."*

The cost shows up as soon as a system's rules depend on what the player has read:

- A debate that gets harder each time you reach it.
- A quiz that only asks the question once the player has heard the answer.
- A puzzle that reuses a clue from three scenes ago.

Each of those can be faked with a counter in `Stats`, and each fake drifts: nothing keeps the
counter in step with the story, so a save, a retry or an edit desynchronises it. That is exactly
the "runtime state shape by luck" this project exists to avoid.

## Options

### A. Keep the contract closed; document the workaround

`Stats` is writable by gameplay and by nothing else, so a system can count its own rounds
perfectly well — as long as the count starts where the system thinks it does. It does not: the
player can reach the node by two paths, restart, or load a save.

Cheap, and wrong for any system whose content depends on history.

### B. Hand the system `&Narrative`

The real thing, read-only. Rejected for a reason that matters more than convenience: a system
holding `&Narrative` can call `advance`, `choose` and `save`, and the boundary between "the
story drives gameplay" and "gameplay drives the story" stops being a rule. A `&` is not a
read-only guarantee in Rust for anything with interior mutability, and it invites exactly the
tangle the contract exists to prevent.

### C. A snapshot: owned, read-only, copied at the handover

`NarrativeSnapshot` with the node, the beat position, the visit history and the choices made.

A system can see where it is. It cannot change anything, and it cannot reach the graph, the stage
or the renderer — the type has no way to. The information flows one way, which is what keeps
"adding a gameplay cannot break the story" true rather than aspirational.

## Decision

**C.** `GameplayContext` gains `narrative: &NarrativeSnapshot`, and `GameplaySystem::begin`
takes the same snapshot.

`begin` gets it because a system that varies its content by context has to choose before the
first frame. A quiz that presents its first question in `begin` and only reads the snapshot in
`step` would show the wrong question for one frame.

```rust
pub struct NarrativeSnapshot {
    pub node: NodeId,
    pub beat_index: usize,
    pub beat_count: usize,
    pub visited: Vec<NodeId>,   // repeats included, oldest first
    pub choices: Vec<ChoiceMade>,
}
```

Two conveniences, because they are the questions actually asked: `visits(node)` and
`has_visited(node)`.

`visited` keeps repeats on purpose. "Is this the second time we are here" is what a boss fight
asks, and a set would answer it wrongly. A caller that wants the other question checks
`contains`.

## History has to survive a save

The snapshot is only honest if it is complete, and a save is where honesty is tested. Both the
visit list and the choice list are stored in `SaveState`, `#[serde(default)]` so a save written
before this existed still loads — it describes a story with no recorded history, which is what
it had.

A system resumed from a save therefore sees the same history the system that wrote the save saw.
The alternative — rebuilding history from config — is the drift this ADR exists to remove.

## What a system still cannot do

- Read the text of a line. Transcripts would tempt a system into regexing dialogue, which is a
  parser living in the wrong place.
- Read another system's state.
- Write anything but `Stats` and its own outcome.
- Ask which language the story is playing in. A gameplay screen is text it produces itself; if
  a system ever needs translated strings, that is a localisation decision of its own, not a
  side effect of this one.

## Consequences

- Adding a gameplay no longer requires predicting every context it will be dropped into; it can
  ask.
- `SaveState` grows by two lists. A long session's save is a few kilobytes larger, which is a
  fair price for a system that resumes correctly.
- `GameplaySystem` grows a parameter on `begin` and a field on the context. Every existing
  implementation had to change — including the two shipped systems and the guide's example — and
  that is the honest cost of a contract that was too narrow.

## Verification

- `crates/rustory-vn-core/tests/gameplay_context.rs` — six tests, including a system that reports
  a different outcome on its second visit, that the visit count and history survive a save and
  resume, and that a save written before the fields existed still loads.
- `apps/rustory-vn/tests/guide_example.rs` — the guide's example, compiled and run, now passing a
  snapshot it does not use; that is the shape most systems will have.
- Both shipped gameplay crates take the snapshot and ignore it, which is also the shape most
  systems will have.
