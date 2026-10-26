# Architecture Decision Records

Each file records one decision that **constrains every subsequent line of code**:
the context, the options weighed, the choice, and what it costs.

The rule: changing the architecture without writing an ADR counts as not having
changed it. An accepted ADR carries a status label. A reversed decision is never
deleted — it becomes `Superseded by NNNN` with the reasoning kept for the record.

| No. | Title | Status | Open question it answers |
|---|---|---|---|
| [0001](0001-engine-foundation.md) | Engine foundation: core in-house, platform layer borrowed | Accepted | — (supersedes the original Bevy ECS plan) |
| [0002](0002-runtime-not-toolchain.md) | The engine is a runtime, not an authoring toolchain | Accepted | Runtime or toolchain? |
| [0003](0003-gameplay-integration.md) | Gameplay integration: trait, registry, opaque config | Accepted, implemented | What shape is the gameplay API? |
| [0004](0004-cross-system-contract.md) | Cross-system contract: events outward, narrow state inward | Accepted, implemented | How do narrative and gameplay share state? |
| [0005](0005-wasm-vs-upper-ui.md) | WASM sandbox vs upper-layer UI: split by what each can break | Proposed | Which gameplay goes where? |
| [0006](0006-save-and-rollback.md) | Save and rollback: snapshots over event replay | Proposed | How does gameplay state enter a save? |

## Status meanings

- **Proposed**: direction settled, not built. Open to revision, but the document
  must be updated with it.
- **Accepted**: direction settled; implementation may or may not exist.
- **Accepted, implemented**: the code enforces it, and changing it breaks tests.
- **Superseded**: no longer in force, kept so the trail stays readable.

## What still needs an ADR

None of these are decided. Writing one is the first step before changing code.

- **Audio**: when to wire it in, `rodio` or `kira`, and whether audio state
  belongs in a save.
- **Gameplay reading narrative state**: does it need a read-only snapshot? See
  the open gap recorded in ADR 0004.
- **Format evolution**: how a script migrates when the story format gains a DSL.
- **Localisation**: externalised text and the font fallback strategy.
