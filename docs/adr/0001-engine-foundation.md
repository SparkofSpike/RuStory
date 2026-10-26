# ADR 0001 — Engine foundation: core in-house, platform layer borrowed

- **Status**: Accepted. **Supersedes** the original "build on Bevy ECS" decision.
- **Date**: 2026-10-26
- **Decided by**: the project owner, who reversed the original constraint while
  the scaffold was being built. This file is the record.

---

## Context

The original project brief listed "primary path: based on Bevy ECS" as a **settled
constraint**, and the reasoning was not unreasonable: ECS separates data from
behaviour, which maps onto "gameplay = an independent System"; an event bus
decouples systems; and it removes a large chunk of rendering infrastructure from
the workload.

On the day the scaffold landed (2026-10-26), the owner objected:

> I don't really want to use another engine. I'm already hand-writing an engine —
> can we not?

That was not an idle preference. It points straight at the project's premise:
RuStory exists because engines cannot cleanly compose heterogeneous gameplay, and
**the selling point is that the boundaries are defined here.**

## Question

Can "narrative is the warp, gameplay is the weft" with verifiable boundaries be
achieved on top of Bevy?

## Options

### A. Build on Bevy ECS (the original plan)

- Saves the renderer, windowing, resource management and scheduler.
- Forces core state onto `Entity` / `World` / `SystemParam`.

### B. Build the core in-house, borrow the platform layer (chosen)

- World and component storage, system registration and scheduling, the narrative
  state machine, the story format, the gameplay contract and saving are all ours.
- Borrow only `winit` (window and input), `wgpu` (GPU backend), and
  `glyphon` + `cosmic-text` + `swash` (text shaping and glyph rasterisation).

### C. Build everything, including windowing and text

- Rejected. Hand-writing a window abstraction and a text shaper means rebuilding
  operating-system abstractions, which has nothing to do with composable
  architecture.

## Decision

**Adopt B.**

## Reasoning

1. **Ownership of the boundary.** Bevy's scheduling model *is* its
   `Entity`/`World`/`System` trio. The moment core state lives on those, "the
   boundaries are mine" becomes "the boundaries are Bevy's" — the exact opposite
   of what this project is trying to prove. The claim to demonstrate is that we
   can define a contract, not that we can configure someone else's framework.
2. **Domain mismatch.** A visual novel needs backgrounds, sprites, text and
   clicks. Bevy's 3D, physics, scene graph and PBR go unused but still enter the
   dependency tree and the mental load.
3. **Saves and rollback.** Snapshotting on ECS requires handling entity
   remapping. In a world we wrote, the state shape is our own `struct`/`enum`, so
   `serde` serialises it directly. **Hand-writing is a net win here, not a
   compromise.**
4. **Version stability.** Bevy is evolving quickly with frequent breaking API
   changes. This project requires stability and readability first; following
   upstream refactors erodes both continuously.
5. **Precedent supports it.** Among reference projects, Raven and Sabi use Bevy;
   Ayaka and MOYU use "Rust core + upper-layer UI"; Amble uses data-driven design
   with a purpose-built compiler. Bevy is evidently not a prerequisite for this
   class of engine.

## Consequences

**Gained**

- Contracts, scheduling and state shape are defined here and can be tightened at
  compile time.
- Saving means serialising our own types; no ECS adapter layer.
- A much smaller dependency tree, faster builds, more predictable behaviour.
- The core ends up with zero rendering dependencies, so it unit-tests in memory.

**Paid**

- The window/input loop, the render pipeline and text layout must be wired up by
  hand — wiring, not reimplementing algorithms.
- The first visible frame arrives later than it would with Bevy. Estimated at one
  to two days.
- Text layout is the one genuine hazard: CJK line breaking, font fallback, and a
  typewriter effect that must split on grapheme clusters. **This part is
  explicitly not hand-written**; it uses `cosmic-text` / `swash`.
- 3D or complex particles later would have to be built rather than borrowed.

**Risks and mitigations**

- Risk: reinventing wheels without end. Mitigation: the dividing line is written
  into `AGENTS.md` §3 — the platform layer is borrowed, never rebuilt.
- Risk: `wgpu` and `glyphon` are version-locked (`glyphon 0.12` requires
  `wgpu ^30.0.0`). Mitigation: both are declared at workspace level, so upgrading
  one forces the other into view.
- Risk: a later contributor restores Bevy by following the old brief. Mitigation:
  this ADR is cited from rule 1 of `AGENTS.md`.

## Verification

Verified after the decision landed:

- `cargo build` succeeds (wgpu 30 + glyphon 0.12 + winit 0.30 compile together).
- `cargo run -p rustory` reports a real GPU adapter and font count:
  `Intel(R) RaptorLake-S Mobile Graphics Controller [Intel Corporation / Vulkan]`,
  `436 system fonts discovered`.

In other words: the chosen platform layer actually works on this machine. It is
not a paper design.
