# ADR 0005 — WASM sandbox vs upper-layer UI: split by what each can break

- **Status**: **Proposed**, not implemented. The WASM tier has since been built — see
  [0015](0015-wasm-tier.md). The decision tree below still stands; the "why not now" section
  does not, and is kept because the reasoning is what changed.
- **Date**: 2026-10-26
- **Open question**: which gameplay goes to WASM, which to an upper-layer UI?

---

## Context

The original constraint said: "isolation first — high-risk or fast-iterating
gameplay should preferentially live in a sandbox or a separate layer (WASM plugins
or an upper JS/React layer)". It never defined **which gameplay counts as
high-risk**.

Separately, ADR 0001 decided the engine core is written in-house rather than on a
third-party engine. That does not rule out WASM: a WASM plugin is a module running
on top of the core, not a framework replacing it.

## The axes that matter

Not "how complex is the gameplay" but two questions:

1. **What can it break?** Does the access this system needs threaten core state or
   the host system?
2. **How fast must it iterate?** Hot reload, editable by a non-programmer, or
   compile once and be done?

## Decision tree

```
Does the new gameplay need to load at runtime (user-installed mods, hot reload,
no engine rebuild)?
├── Yes ──▶ Does it need filesystem, network or native API access?
│           ├── Yes ──▶ Refuse. Denying those is the whole point of a sandbox.
│           │            Demote it to a compile-time crate.
│           └── No  ──▶ [WASM plugin]
└── No  ──▶ Is it heavily UI-driven (Live2D-grade animation, complex forms,
            rich text editing)?
            ├── Yes ──▶ [Upper-layer UI] (Tauri + web front end)
            └── No  ──▶ Can its state be expressed with Stats plus its own data
                        types?
                        ├── Yes ──▶ [Compile-time crate] (the ADR 0003 form)
                        └── No  ──▶ Change the contract and write an ADR first.
                                     Punching a hole is not an option.
```

## The three tiers

### Compile-time crate (the only tier implemented today)

- Example: `rustory-vn-gameplay-timer`.
- Trusted, compiled with the engine, zero runtime overhead, full access to Rust.
- **Almost all gameplay should stay here.** A sandbox is a cost, not a badge.

### WASM plugin (proposed)

- For third-party mods, hot reload, and creators writing their own gameplay.
- Constraints:
  - Receives only a serialised `GameplayContext` (time, input, a `Stats` view).
  - Can return only an `OutcomeKey`.
  - No filesystem, network or clock access; time is passed in by the host.
  - A step limit, so one infinite loop cannot hang the host.
- Costs: serialisation across the boundary, harder debugging, error messages
  swallowed by the sandbox.
- **Not recommended for the MVP.** Without real third-party mod demand it only adds
  surface area to maintain.

### Upper-layer UI (proposed)

- For heavy visuals or interaction where rewriting in Rust does not pay off.
- The difference from WASM: this is **not gameplay logic**, it is gameplay's
  **presentation layer**. Logic stays in Rust; the UI draws and collects intents.
- Boundary: only "snapshot down, intents up" between UI and core — the same shape
  as the existing `Frame`.

## Why not now

1. There is no real need. One gameplay system exists, and a compile-time crate is
   entirely sufficient.
2. A sandbox's value depends on the premise of **untrusted code**. Gameplay we
   write ourselves is trusted.
3. Introducing WASM early would turn `GameplaySystem` from a trait into a
   cross-boundary protocol, which would contaminate the clean contract ADR 0003
   established.

## Triggers to revisit this ADR

Restart the discussion when any one of these becomes true:

- There is demand for player- or third-party-installable gameplay packages.
- There is demand to change gameplay without rebuilding the engine.
- More than five gameplay systems exist and one has a markedly heavier dependency
  footprint than the rest, which is a signal that it wants isolation.

## Still undecided

- WASM runtime choice (wasmtime vs wasmer) has not been investigated.
- The cross-boundary data format (JSON / RON / bincode) is not settled.
- Whether plugins should get a restricted "read a script node" capability is
  undecided.
