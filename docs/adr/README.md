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
| [0006](0006-save-and-rollback.md) | Save and rollback: snapshots over event replay | Accepted, implemented | How does gameplay state enter a save? |
| [0007](0007-fonts-are-the-projects-choice.md) | Fonts are the project's responsibility, not the engine's | Accepted, implemented | Where do typefaces come from? |
| [0008](0008-editor-architecture.md) | The editor is an operation layer with two front-ends | Accepted, implemented | How do a GUI and an agent edit the same story? |
| [0009](0009-audio-as-beats.md) | Sound is a beat; a one-shot is an event, music is state | Accepted, implemented | When and how does the engine make noise? |
| [0010](0010-multiple-documents.md) | Several documents, one window | Accepted, implemented | How are two chapters open at once? |
| [0011](0011-localisation.md) | Text has a name, and the engine resolves it | Accepted, implemented | How does a story ship in three languages? |
| [0012](0012-gameplay-sees-the-story.md) | A gameplay system can see where the story is | Accepted, implemented | Does gameplay need read-only narrative state? |
| [0013](0013-gameplay-draws-itself.md) | A gameplay system draws its own screen | Accepted, implemented | How does a board, a cursor or a bar get on screen? |
| [0014](0014-two-ways-back.md) | Two ways back: save slots for players, a timeline for authors | Accepted, implemented (timeline) | How does someone return to an earlier state? |

## Status meanings

- **Proposed**: direction settled, not built. Open to revision, but the document
  must be updated with it.
- **Accepted**: direction settled; implementation may or may not exist.
- **Accepted, implemented**: the code enforces it, and changing it breaks tests.
- **Superseded**: no longer in force, kept so the trail stays readable.

## What still needs an ADR

None of these are decided. Writing one is the first step before changing code.

- **Pointer input**: a system can draw a board but cannot be told where the player clicked.
  Input is still `Confirm` and `Select { index }`. Needs an ADR before code.
- **Persisting the editing timeline**: history is in memory only. A named checkpoint file is
  the obvious next step; where it lives and how it interacts with git is undecided.
- **Format evolution**: how a script migrates when the story format gains a DSL.
- **WASM sandbox**: ADR 0005 is a proposal with no code behind it.
