# RuStory

**A Rust-native visual novel engine with composable gameplay.**

RuStory is a visual novel engine built around one idea: the narrative is the
warp, gameplay is the weft. Adding a new gameplay system should mean weaving in
another thread — not tearing the cloth open and patching it back together.

> Not an official Rust project. Not affiliated with or endorsed by the Rust
> Foundation. The name contains "Rust" for the language it is written in; the
> Rust name and logo policy applies to the Rust project itself, not to this
> engine.

---

## Why

Most visual novel engines are narrative-centric: a single event loop plays a
script, and anything that is not dialogue — combat, puzzles, management — has to
be bolted on from the outside. The narrative system and the gameplay system then
share state with no contract the compiler can check, and the result is timing
bugs and shape mismatches discovered at runtime.

RuStory takes the opposite position:

- **Explicit boundaries.** Cross-system state is modelled with `enum`, `trait`
  and explicit events. What the compiler can reject, we never leave to runtime.
- **Data and behaviour are separate.** A gameplay system reads and writes only
  the state it declares. It talks to the narrative system through a small,
  named contract.
- **Extension is addition.** A new gameplay system registers itself. It does not
  edit the narrative core.
- **Readability is a first-class requirement.** No implicit global mutable
  state, no dynamically shaped bags of values.

## Status

**Scaffold.** This repository is at the very beginning. What currently works:

```console
$ cargo run -p rustory
rustory 0.1.0 — scaffold only, no playable slice yet
core:   rustory-core
script: rustory-script
gpu:    Intel(R) RaptorLake-S Mobile Graphics Controller [Intel Corporation / Vulkan]
text:   436 system fonts discovered
```

That is a real GPU adapter probe and a real font-stack probe — it verifies the
rendering foundation is usable on the host machine. There is **no playable
slice yet**: no story format, no state machine, no dialogue rendering.

Everything below in [Architecture](#architecture) describes intended structure,
not shipped behaviour. See [`HANDOFF.md`](HANDOFF.md) for the task sequence and
[`docs/adr/`](docs/adr/) for decisions made so far.

## Architecture

The engine is written directly against the platform, without a third-party game
engine underneath. The split is: **game semantics are ours, operating-system
abstractions are borrowed.**

| Layer | Crate | Owns |
|---|---|---|
| Narrative core | `rustory-core` | World and component storage, system scheduling, narrative state machine, gameplay contract |
| Story format | `rustory-script` | RON schema, loading, referential validation |
| Renderer | `rustory-render` | 2D quad and text pipeline on `wgpu` |
| Application | `apps/rustory` | `winit` event loop, CLI |

Borrowed from the platform, deliberately not reinvented: `winit` (window and
input), `wgpu` (GPU backend), `glyphon` / `cosmic-text` / `swash` (text shaping
and glyph rasterization), `rodio` or `kira` (audio, not yet wired).

`rustory-core` has **zero rendering dependencies** and must stay testable
without a window or a GPU. Dependencies point one way: `render → core`,
`script → core`, `app → all`. The core never depends upward.

## Building

Requires Rust 1.85+ (edition 2024) and, on Windows, the MSVC toolchain.

```console
cargo check --workspace     # fastest feedback
cargo build                 # first run pulls the wgpu dependency tree; minutes
cargo run -p rustory        # platform probe
cargo test --workspace      # core logic, no GPU required
```

The dev profile builds dependencies at `opt-level = 3` (an unoptimized
`wgpu`/`naga` build is slow enough to make frame-rate problems unreadable) while
keeping workspace crates at `opt-level = 1` for incremental builds.

## Contributing

Read [`AGENTS.md`](AGENTS.md) first — it is the operating standard for this
repository, and it is written for both humans and coding agents. The four rules
that matter most:

1. The engine is ours; no third-party game engine.
2. Extension is addition; a new gameplay system never edits the core.
3. Boundaries are verifiable; prefer compiler errors over runtime surprises.
4. No unverified claims in documentation.

Commits follow Conventional Commits in English, one logical change per commit.

## License

MIT OR Apache-2.0, at your option.
