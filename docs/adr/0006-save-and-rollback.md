# ADR 0006 — Save and rollback: snapshots over event replay

- **Status**: Accepted, implemented
- **Date**: 2026-10-26, revised 2026-10-27 after implementation
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
  `HashMap` iteration order, one future random number and the replay diverges. With
  third-party gameplay systems this is impossible to hold, and a save that diverges
  silently is worse than one that is a few kilobytes larger.

### B. State snapshot

Serialise `Narrative`'s complete state.

- Good: loading restores everything, with no dependence on history.
- Bad: larger saves; each gameplay system needs a way to serialise itself.

### C. Snapshot plus a ring of history

Current state as a snapshot; rollback via the last N lightweight snapshots.

## Decision

**Adopt C: B as the foundation, with rollback as a ring of snapshots.**

For the gameplay half, **the trait gains two methods** rather than requiring
systems to compress their state into shared counters.

## Why the trait, and not "rebuild from Stats"

The first draft of this ADR preferred rebuilding gameplay state from `Stats`, on
the grounds that a new system then would not have to implement anything. That was
rejected after the second gameplay system was written.

A quiz holds a growing list of answers and a cursor into a list of rounds. It
cannot be rebuilt from `Stats`, and trying would mean inventing an encoding on top
of a map of integers — exactly the dynamically shaped improvisation this project
exists to avoid. More importantly, a rule that works only for simple systems
contradicts the goal of being an engine other people build on: the first person to
write something stateful would hit a wall the architecture put there.

So `GameplaySystem` gained:

```rust
fn save(&self) -> Result<Option<ron::Value>, CoreError> { Ok(None) }
fn restore(&mut self, state: Option<&ron::Value>) -> Result<(), CoreError> {
    match state {
        None => Ok(()),
        Some(_) => Err(/* refuses state it cannot use */),
    }
}
```

Both have defaults, so neither breaks existing systems, and neither forces a
stateless system to write boilerplate. But a system that needs state can carry
anything it can serialise.

The default `restore` **rejects** state rather than ignoring it. A save that
carries state for a system that never learned to restore it means the save and the
code disagree; silently discarding it would resume the scene with invisible
progress lost.

## What lives in a save

```
SaveState {
    version: u32,                    // format version, checked on load
    story_checksum: u64,             // FNV-1a over the story's RON form
    cursor: NodeId,
    beat_index: usize,
    mode: Mode,                      // Revealing / AwaitingAdvance / Choice / Suspended / Finished
    revealed: usize,                 // grapheme clusters of the current line
    stage: Stage,                    // background and actors
    stats: Stats,
    gameplay: Option<GameplaySave>,  // { system, state: Option<ron::Value> }
}
```

`version` and `story_checksum` are what make a load safe. Without the checksum,
loading a save against an edited script resumes at a node whose beats have shifted
under the player, which surfaces as a scene that makes no sense rather than as an
error.

## Validation on load

Every check exists because the alternative is a silent mis-restore. A refused load
is recoverable; a corrupted one looks like an engine bug.

| Check | Failure |
|---|---|
| Version matches | `SaveVersionMismatch` |
| Story checksum matches | `SaveStoryMismatch` |
| Node exists | `SaveNodeMissing` |
| Beat index within the node | `SaveBeatOutOfRange` |
| Mode fits the node's exit (a pending choice needs a `Choose` exit, a suspension needs a matching `Gameplay` exit) | `SaveBeatOutOfRange` / `SaveGameplayMismatch` |
| Suspended mode carries gameplay state | `GameplayRestoreFailed` |
| Non-suspended mode carries none | `GameplayRestoreFailed` |

One deliberate exception: a `revealed` count beyond the current line is **clamped**
rather than rejected. The worst case is a line that shows slightly less than it did
before, which the player cannot distinguish from a slow reveal — refusing would
throw away a playthrough over a cosmetic difference.

## Consequences

**Gained**

- Load reliability does not depend on whether a system is deterministic.
- Gameplay systems store arbitrary state without the core learning about them.
- Saves are readable RON, so "it restored into the wrong scene" is answerable by
  reading the file.

**Paid**

- Save size grows with the number of systems and the stage.
- The `version` field and migration logic must be maintained.
- `GameplaySystem::save` returning `None` for a system that *does* hold hidden
  state fails silently — the save simply resumes with a fresh system. The doc
  comment states the rule plainly: if `begin` cannot reproduce it, it belongs in
  `save`.

**Explicitly not done**

- Rollback history. The snapshot type supports it, but no ring buffer exists yet;
  rolling back needs UI for choosing a point, not just storage.
- Cross-story state inheritance. That needs an explicit migration language.

## Verification

Implemented and covered by tests:

- `crates/rustory-vn-core/tests/save.rs` — 15 tests: mid-line resume, stage and
  counters, pending choice, finished story, checksum and version rejection, beat
  and mode mismatches, reveal clamping, stable serialisation, and a gameplay
  system whose list state survives a save.
- `crates/rustory-vn-gameplay-quiz` — the system that forced this design, with its own
  tests for a half-finished save resuming at the right round and for rejecting a
  round index beyond the config.
- End to end through the CLI: `rustory save stories/demo.ron --advance 6` writes a
  save suspended in the timed choice with `remaining_ms: 5000`; `rustory resume
  ... --finish` reaches the same ending as playing straight through.
