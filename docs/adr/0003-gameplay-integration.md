# ADR 0003 — Gameplay integration: trait, registry, opaque config

- **Status**: Accepted, implemented. Contract widened since: [0012](0012-gameplay-sees-the-story.md)
  added a read-only narrative snapshot, [0013](0013-gameplay-draws-itself.md) replaced the
  single-string prompt with a structured view.
- **Date**: 2026-10-26
- **Open question**: what shape should the gameplay integration API take?

---

## Context

"Adding gameplay is adding a thread" is the first sentence of this project. That
sentence is either true or it is a slogan. The difference comes down to one
question: **when you add a gameplay system, which files change?**

If the answer is "add a variant to the core enum", the slogan is false — because
every new gameplay system edits the core, and the core eventually becomes a
cupboard where every gameplay system stores its junk.

## Options

### A. The core enum lists every gameplay system

```rust
pub enum GameplaySpec {
    Debate { rounds: u32 },
    Puzzle { grid: Vec<u8> },
}
```

- Good: fully type-safe at compile time.
- Bad: **every new system edits the core**, and the core must depend on every
  system's config type. This directly contradicts the project's goal.

### B. Fully dynamic: `HashMap<String, Box<dyn Any>>`

- Good: the core knows nothing about any system.
- Bad: config shape is a runtime gamble — the exact structural defect this
  project was founded to remove.

### C. Trait-object registry with an opaque config

```rust
pub trait GameplaySystem {
    fn id(&self) -> SystemId;
    fn begin(&mut self, config: Option<&ron::Value>) -> Result<(), CoreError>;
    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey>;
    fn cancel(&mut self);
    fn prompt(&self) -> Option<String> { None }
}
```

The core recognises three things only: a system id, a config it **never parses**,
and an outcome key it **never interprets**. Where each outcome leads is decided by
the script's `outcomes` table.

## Decision

**Adopt C.**

## Reasoning

1. **Type safety is pushed inside each system's boundary.** The core side is weakly
   typed (`ron::Value`), but each system deserialises into its own strong type at
   the top of `begin` and **refuses the handover if validation fails**. The loss of
   type safety is confined to one method rather than spread across the core.
2. **Outcome meaning belongs to the script, not the code.** Where `correct` leads
   is a property of that scene. The same system can have entirely different
   consequences in different scenes with no code change.
3. **There is exactly one erased seam, four methods wide.** This is not
   "dynamically shaped improvisation": the erased surface is enumerable,
   documented and covered by tests.
4. **Additive extension can be verified mechanically.** See the verification
   section.

## Consequences

**Gained**

- A new gameplay system is a new crate plus one registration line. Zero core
  changes.
- Systems test independently: construct a `GameplayContext` and step it. No story,
  no window, no GPU.
- Systems are mutually unaware; ordering is the script's business.

**Paid**

- A bad config surfaces **when playback reaches that node** (`begin` fails), not at
  load time. Mitigation: `rustory check` warns when a script names a system that is
  not registered; a pre-flight that dry-runs every `begin` could be added later.
- `ron::Value` leaks into the core's public API. Accepted: it is a data-format
  type, not a gameplay type.
- The registry is a `BTreeMap<SystemId, Box<dyn GameplaySystem>>`, so one id means
  one instance. A pool of instances would only be introduced if parallel copies
  were ever needed.

**Explicitly not done**

- No universal back door such as `&mut World`. A system receives `GameplayContext`
  (time, input, `Stats`) and nothing more. Needing more shared state is a contract
  expansion, and that requires an ADR.

## Verification

`rustory-vn-gameplay-timer` is the evidence for this decision:

- It is its own crate, depending only on `rustory-vn-core`.
- Nothing in `rustory-vn-core` mentions timers, options or correctness.
- Integration is one line in `driver.rs`:
  `registry.register(Box::new(TimedChoice::new()))`.
- It is unaware of the stage, the story and rendering — none of those are in
  `GameplayContext`.

Put plainly: **the thread was added without disturbing the cloth.**
