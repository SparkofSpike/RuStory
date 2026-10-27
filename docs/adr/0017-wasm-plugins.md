# ADR 0017 — A story names its modules; the environment supplies them

- **Status**: Accepted, implemented
- **Date**: 2026-10-27
- **Open question**: how does a script reach a WebAssembly module?

---

## Context

ADR 0015 shipped a WebAssembly tier that works: `WasmGameplay::open(path)` returns a
`GameplaySystem`, the guest gets a narrow ABI, and a module that loops is stopped by fuel.

It left the question a creator actually asks: **how does the script name it?** The first
implementation answered "it does not" — `id()` returned a constant `"wasm.plugin"`, which meant a
project could have exactly one module, and loading a second would silently replace the first in
the registry. A story would run another module's rules and nothing would say so.

## Options

### A. The path lives in the script

```ron
exit: Gameplay(spec: (system: "wasm.puzzle", …))
```

with a story-level `plugins: { "wasm.puzzle": "mods/slide.wasm" }`.

Portable: the mapping travels with the file. Also puts an environment fact — where a binary sits
on this machine — into a document that is meant to be committed and shared.

### B. The path comes only from the command line

```
rustory play story.ron --plugin wasm.puzzle=mods/slide.wasm
```

The script stays clean, and a different build can be swapped in without touching the story. Also
means a cloned repository does not play: the command has to be reconstructed from a README.

### C. Both, plus a convention

The script may declare a path, the command line may override it, and a `plugins/` directory is
searched when neither says anything.

## Decision

**C**, with the resolution order fixed and documented:

1. `--plugin name=path` — a person typing it is trying to run a specific build, so it wins.
2. The story's `plugins` map — the author's recorded default, resolved relative to the story file
   (not the working directory, because a story opened from elsewhere must still find its modules).
3. `plugins/<name>.wasm` or `plugins/<name>/plugin.wasm`, beside the story **or one level up** —
   both, because both are what projects actually do: one directory per chapter's modules, or one
   directory for the whole project.
4. Nothing. Which is not an error *here*: the name may belong to a compiled-in system, and this
   layer has no way to tell. `rustory check` reports what is still unregistered, which is where
   the answer belongs.

### The name is the contract; the path is only how it was found

`WasmGameplay::with_id(name, bytes)` takes its id from the caller. `open` keeps the old
single-argument form for tests. A guest does not know or care what it is called — which is what
lets a project pin `wasm.puzzle` to a debug build locally and a release build in CI without
touching the story.

### A compiled-in system is not replaceable by a file

If a name is already in the registry, a module is not loaded over it. The engine's own systems are
a promise; a `.wasm` file with the same id is not entitled to take the name.

### A module that fails to load is fatal, not skipped

The path was declared, so the author meant it to exist. Loading nothing would leave the story
unplayable with no explanation, so `play`, `shot`, `transcript`, `save`, `resume` and `check` all
fail with the module's name and the loader's reason.

## Consequences

- `Story` gains `plugins: BTreeMap<SystemId, String>`, `#[serde(default)]`, so every existing
  script parses unchanged.
- `rustory check` reports plugins in its JSON under `plugins.loaded` / `plugins.failed`, including
  **which source** supplied each one. "It loaded the wrong build" is the failure this description
  exists to make diagnosable.
- `--plugin` is extracted before each command's own option parsing, so it works in any position
  rather than only where someone remembered to add a branch.
- Five commands call one `registry_for` helper, so "which systems exist" has one answer rather
  than one per command.

## The two things measuring this taught

### 4 096 instructions was not a budget, it was a bug

The first `DEFAULT_FUEL` was 4 096, chosen when the only guests were hand-written WAT modules that
did nothing. A real Rust guest — `examples/wasm-guest`, no dependencies, whose `begin` only scans
the payload for one number — needs **between 4 400 and 4 600** instructions for its first call. It
failed at its first handover, with the engine reporting "out of fuel" on a module that had done
nothing wrong.

The constant is now 16 000, and both bounds are measured:

- floor: 4 400–4 600 for a real Rust `begin` (bisected against the example guest)
- ceiling: wasmi 2.0 runs wasm frames on the host's Rust stack, so fuel is not what stops a
  runaway guest — the stack wins first. 16 000 survives a 2 MB thread; 32 000 overflows it.

So the number sits just under the limit it can actually enforce, and ADR 0015's claim that a
guest gets "roughly four thousand instructions per frame" is corrected there. A guest that needs
more needs a worker thread with a larger stack, not a bigger constant.

### A bad view was invisible

`GameplaySystem::view` returns `Option`, so the wasm implementation's `.ok()?` turned a malformed
RON view into `None` — indistinguishable from "this guest draws nothing". A hand-written guest
that emitted `Panel:(…)` instead of `Panel(…)` rendered a blank screen and said nothing.

Caught during the end-to-end work, and fixed two ways:

- The failure is now printed: `rustory: plugin `doors` returned an unusable view: …`.
- `crates/rustory-vn-core/tests/view_shape.rs` pins the exact RON a guest must write, including
  that `Panel:(` is invalid, so the shape cannot drift without a test failing.

## Verification

- `apps/rustory-vn/src/plugins.rs` — 9 unit tests: `name=path` parses at the first `=`, a path may
  contain one, malformed flags are refused, `wanted` is sorted and deduplicated across invocations
  and declarations, a flag beats the story and the directory, a story path resolves relative to
  the story, an absolute path is taken as written, an unknown name resolves to nothing, a missing
  declared module is reported rather than skipped, and a compiled-in system is not replaced.
- `crates/rustory-vn-core/tests/view_shape.rs` — 4 tests pinning the guest-facing RON.
- End to end against a real module, not a fixture: `examples/wasm-guest` is a `no_std` Rust crate
  built for `wasm32-unknown-unknown` (1 633 bytes), copied to `plugins/doors.wasm`, and played by
  `stories/doors.ron`:
  - `rustory check stories/doors.ron` → `plugin doors: plugins\doors.wasm (directory)`, no
    complaints
  - `rustory transcript stories/doors.ron` → the guest decides, the story takes the `won` branch
  - `rustory shot stories/doors.ron --at hallway:exit` → `16 primitives`,
    `shots/wasm-gameplay.png`, rendered and inspected
