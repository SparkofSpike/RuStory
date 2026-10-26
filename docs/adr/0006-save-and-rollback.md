# ADR 0006 — Save and rollback: snapshots over event replay

- **Status**: **Proposed**, not implemented
- **Date**: 2026-10-26
- **Open question**: how does gameplay state enter a save?

---

## Context

Saving with composable gameplay is harder than in a plain visual novel:

- A save must carry both **narrative position** (which node, which beat, reveal
  progress) and **gameplay state** (how long the timer has left, what the board
  looks like).
- After loading, a gameplay system has to resume where it stopped, not restart.
- Rollback — the visual-novel convention of "back to the last choice" — needs
  **historical snapshots**, not just the current one.

The in-house core from ADR 0001 pays off directly here: the state is our own
`struct`, so there is no ECS entity remapping to handle.

## Options

### A. Event sourcing

Record every input event and replay from the beginning on load.

- Good: tiny saves; rollback to any moment comes for free.
- Bad: **replay must be perfectly deterministic.** One accumulated `f32` error, one
  `HashMap` iteration order, one future random number and the replay diverges. The
  more gameplay systems exist, the harder determinism is to hold. For composable
  gameplay this is a trap.

### B. State snapshot

Serialise `Narrative`'s complete state directly.

- Good: loading restores everything, with no dependence on history; results are
  necessarily identical.
- Bad: larger saves; each gameplay system needs its own serialisation format.

### C. Snapshot plus a ring of history

Current state as a snapshot; rollback via the last N lightweight snapshots.

## Decision

**Adopt C: B as the foundation, with a ring buffer of historical snapshots for
rollback.**

## Reasoning

1. **Determinism is a scarce resource, and snapshots do not need it.** Writing
   `SystemTime::now()`, iterating a `HashMap`, or accumulating floats all come
   naturally inside a gameplay system. Event replay would forbid all of it and
   require review to keep it forbidden — far more expensive than a few extra
   kilobytes.
2. **Players notice load reliability, not save size.** One intermittent
   mis-restore ruins the experience; a save that is 20KB larger goes unnoticed.
3. **Rollback only needs a few steps, not all of history.** Visual novel rollback
   typically reaches the previous choice, so keeping the last 16 choice-point
   snapshots is enough.

## What has to be serialised

```
SaveState {
    version: u32,                    // format version, mandatory
    story_id: String,                // story identity plus checksum
    cursor: NodeId,
    beat_index: usize,
    mode: SavedMode,                 // Revealing/AwaitingAdvance/Choice/Suspended/Finished
    revealed: usize,                 // reveal progress, so loading resumes mid-line
    stage: { background, actors },
    stats: BTreeMap<String, i32>,
    gameplay: Option<GameplaySave>,  // see below
}
```

### The hard part: gameplay state

`GameplaySystem` is a trait object, and the core has no idea how to serialise it.

Two approaches:

- **Approach 1**: add `fn save(&self) -> ron::Value` and
  `fn restore(&mut self, value)` to the trait. Cost: every system implements it, in
  a format of its own choosing.
- **Approach 2**: **rebuild from the narrative side.** The save records only "which
  node was suspended plus `Stats`"; on load the system is `begin`-ed again and
  progress is restored from `Stats`.

**Approach 2 is preferred**, because it stays additive — a new system does not have
to implement a save interface. The price is that a system must be reconstructible
from `Stats`. The timed choice satisfies this: remaining time follows from an
`elapsed_ms` counter.

**But there is a hard limit**: if a system's state cannot be compressed into
`Stats` — a randomly generated board, say — approach 2 does not hold. At that point
approach 1 becomes mandatory, and **the ADR must explain why that system needs its
own save format**.

This is a genuine open question, not something to paper over.

## Consequences

**Gained**

- Load reliability does not depend on whether a system is deterministic.
- Rolling back to the previous choice is an O(1) restore.

**Paid**

- Save size grows with the number of systems and the size of the stage.
- The `version` field and migration logic must be maintained. **Rejecting a version
  mismatch on load is safer than guessing.**
- `story_id` plus a checksum is mandatory: loading an old save against a new script
  would otherwise misalign silently.

**Explicitly out of scope**

- Cross-story state inheritance. That needs an explicit migration language, far
  beyond the MVP.

## Still undecided

- Format: RON (readable, diffable, debuggable) vs bincode (small, fast).
  **RON is preferred**, because saves are investigated by hand far more often than
  they are performance-critical.
- How many historical snapshots to keep: 16 tentatively, pending a memory
  measurement.
- Whether to record "text already seen" for skip-read mode. It is standard for the
  genre but significantly enlarges the save (a node-by-beat bitmap). Suggestion:
  keep it in a separate file, decoupled from the main save.
