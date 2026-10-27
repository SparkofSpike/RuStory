# ADR 0015 — A third tier: gameplay that arrives as WebAssembly

- **Status**: Accepted, implemented (runtime and contract; no CLI flag yet)
- **Date**: 2026-10-27
- **Supersedes**: the "not recommended for the MVP" verdict in ADR 0005, not its decision tree

---

## Context

ADR 0005 described three tiers and implemented one: a compile-time crate. It left WASM as a
proposal with a trigger list — demand for installable gameplay, demand to change gameplay
without rebuilding the engine — and said plainly that it was not worth building early.

The project owner has since decided it is worth building, for a reason the trigger list did not
name: an engine is a thing other people use. A creator who cannot ship a gameplay system without
a Rust toolchain is not using an open engine, whatever the licence says. The trigger list was
written from the engine's side; this is the same decision seen from the creator's.

## Options

### A. Stay compile-time only

Nothing to maintain. Also means every gameplay author must compile the engine, which excludes
most of the people a visual novel engine is for.

### B. wasmtime

The reference implementation, mature, with epoch interruption that stops a runaway loop without
a per-instruction budget. Costs: a dependency tree in the hundreds of crates, a build that
dominates CI, and a substantial amount of surface the engine does not need.

### C. wasmi

A register-machine interpreter in pure Rust, spec-compliant, audited twice, with fuel metering.
Small dependency tree, no JIT, no unsafe in our own code. Costs: slower, and — measured here —
it executes wasm frames on the host's Rust stack, so the fuel budget interacts with stack depth.

### D. No sandbox; run plugins as native libraries

Fastest and the least portable. A native plugin can call anything, which is the opposite of what
this tier is for.

## Decision

**C**, with the boundary kept as narrow as the trait already was.

```text
memory                       the linear memory
alloc(len) -> ptr            a buffer the host may write into, -1 on failure
dealloc(ptr, len)
begin(ptr, len) -> status    0 = ok; payload is RON { config, narrative }
step(dt, ptr, len) -> status 0 = keep going, 1 = finished
result_ptr() / result_len()  where the outcome, view or saved state lives
cancel()
view() -> status             optional: 1 = a GameplayView in RON
save() -> status             optional: 1 = a state in RON
restore(ptr, len) -> status  optional: 0 = ok
```

### No imports at all

Not "WASI with a permission list" — zero imports. The `Linker` defines nothing, so a module that
imports `env.open_file` fails to instantiate. That is stronger than a check, because there is
nothing to check, and it is the reason a host function was not added for logging: the first host
function is the first thing to audit.

Time arrives as `dt_ms`, so a guest cannot observe wall-clock time. Randomness is the guest's
problem; a system that wants a shuffled deck should derive it from its own seeded state, which is
also what makes its save reproducible.

### RON across the boundary

The engine already ships RON: story files, configs, saved gameplay state. Adding JSON would mean
two textual formats and two parsers for the same job. `GameplayView` gained `Serialize` /
`Deserialize` for this — it is plain data, and the editor benefits from being able to round-trip
one.

### Fuel is 16 000 instructions per call, and both bounds are measurements

**Corrected after a real guest failed.** This ADR first set the budget at 4 096, from an experiment
with hand-written WAT modules that did nothing. A Rust guest with no dependencies needs over
4 400 instructions just to log its first call, so every real module failed at its first handover.
The history and the bisection are in [ADR 0017](0017-wasm-plugins.md).

wasmi 2.0 executes wasm frames on the host's stack, so fuel is not the only thing that stops a
runaway guest — the stack wins first. Measured: 16 000 survives a 2 MB thread, 32 000 overflows
it. The constant sits just under the limit it can actually enforce.

The consequence is honest: a wasm system gets about sixteen thousand instructions per frame,
roughly four times what a hand-written guest spends on its first call and far more than a state
machine needs per step. That is enough for turn-based or state-machine gameplay and not enough
for a physics simulation. A system that needs more needs either a worker thread with a larger
stack or a runtime with its own stack management — an ADR of its own, not a bigger constant.

The limit is also a security property: it bounds the work one frame can demand, so a malicious
module cannot freeze the window even if it never touches anything it should not.

### Optional exports, matching the trait's defaults

`view`, `save` and `restore` may be omitted. Absence means "nothing to draw", "no state" and
"cannot restore" — exactly what a Rust system inherits from `GameplaySystem`'s defaults. A guest
that exports `save` but not `restore` gets an error when a save is loaded, because that pair
disagreeing is a bug in the module, not a preference.

## What this does not do

- **No CLI flag yet.** The runtime is a library: `WasmGameplay::open("path.wasm")` returns a
  `GameplaySystem`. Registering it by name from `rustory play --wasm …` is mechanical work that
  waits for a real module to load; inventing the flag before the first plugin exists is the
  speculative interface this project avoids.
- **No guest SDK.** The ABI is documented above and small enough to implement by hand; a Rust
  crate wrapping it is worth writing once two guests exist, not before.
- **No filesystem access, ever.** A module that needs art names it in its view; the project's
  asset bank resolves it. That is the same rule a Rust system follows.

## Consequences

- `rustory-vn-gameplay-wasm` is the fifth gameplay crate and the first that does not contain a
  gameplay system's rules — it contains the boundary those rules arrive through.
- `NarrativeSnapshot`, `GameplayInput` and the view types gained `Serialize`/`Deserialize`. Those
  are inert additions to types that were already plain data.
- wasmi 2.0 is a new workspace dependency, and the first one that executes code the project did
  not write.
- A guest's mistakes are its own: a panic in wasm is a trap the host reports as
  `CoreError::GameplayFailed`, with the module's text kept in the message.

## Verification

- `crates/rustory-vn-gameplay-wasm/tests/guest.rs` — 8 tests against hand-written WAT modules:
  a counter that reports an outcome on its third step and keeps reporting it; a guest with no
  `view` drawing nothing instead of failing; a guest that returns an (empty) view; a save state
  round-tripping as RON; `restore` refused when the guest cannot restore; an infinite loop
  stopped by fuel rather than hanging; a module missing `step` failing to load with the export
  named; and `cancel` returning the system to unstarted.
- The infinite-loop test is also the regression test for the fuel constant: if wasmi's stack
  behaviour changes, that test overflows and says so.
