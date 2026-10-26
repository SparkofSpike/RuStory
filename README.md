# RuStory

**A Rust-native visual novel engine with composable gameplay.**

RuStory is a visual novel engine built around one idea: the narrative is the
warp, gameplay is the weft. Adding a new gameplay system should mean weaving in
another thread — not tearing the cloth open and patching it back together.

> Not an official Rust project. Not affiliated with or endorsed by the Rust
> Foundation. The name contains "Rust" for the language it is written in.

![Dialogue](shots/dialogue.png)

---

## The claim, and how it is tested

The whole design rests on one falsifiable statement:

> Adding a gameplay system does not change the narrative core.

`rustory-core` has no mention of timers, options, rounds, scores or correctness.
`crates/rustory-gameplay-timer` implements all of that in its own crate, parses
its own config, and reports outcome keys that the *script* maps to nodes.

The entire integration is one line:

```rust
registry.register(Box::new(TimedChoice::new()));
```

If adding a system ever requires editing `rustory-core`, the contract is wrong
and the contract gets fixed — not the core.

## Status

**Playable vertical slice.** Working today:

- Load a story from RON, with reference checking before the first frame
- Background, sprites and typewriter dialogue
- Click to advance; first click completes the line, second advances
- Branching choices
- Gameplay handover: a timed choice takes control, then returns it
- `rustory play` in a window, or `rustory shot` to render a frame headlessly
- 69 tests, including offscreen GPU renders that assert where things land

Not yet: audio, save/load, WASM plugins, an authoring tool. Those have ADRs
describing the plan, not code pretending to be one.

## Try it

```console
$ cargo run -p rustory -- gen-assets assets      # placeholder art
$ cargo run -p rustory -- check stories/demo.ron # parse + validate
$ cargo run -p rustory -- play stories/demo.ron  # play it
$ cargo run -p rustory -- shot stories/demo.ron --out frame.png --advance 4
```

The demo runs without a window too:

```console
$ cargo run -p rustory -- transcript stories/demo.ron
放学后的教室，只剩下两个人。
Yuki: 你还没走？
Kai: 在等你。
...
```

## Screens

| Dialogue | Choice | Gameplay |
|---|---|---|
| ![dialogue](shots/dialogue.png) | ![choice](shots/choice.png) | ![gameplay](shots/gameplay.png) |

The art is generated procedurally by `rustory gen-assets` — no binaries in the
repository, and the code that draws it is reviewable.

## Architecture

The engine is written directly against the platform, without a third-party game
engine underneath. **Game semantics are ours; operating-system abstractions are
borrowed.**

| Layer | Crate | Owns |
|---|---|---|
| Narrative core | `rustory-core` | Story model, state machine, gameplay contract. Zero rendering dependencies |
| Story format | `rustory-script` | RON loading, reference validation |
| Renderer | `rustory-render` | Draw list → pixels on `wgpu`. Knows nothing about stories |
| Gameplay | `rustory-gameplay-timer` | One system, proving the extension model |
| Application | `apps/rustory` | CLI, layout, window, input |

Dependencies point one way: everything depends on `rustory-core`; the core
depends on nothing above it.

Three boundaries are enforced by types rather than convention:

1. **The core never parses a gameplay config.** A `GameplaySpec` carries an
   opaque `ron::Value`; each system parses its own.
2. **A gameplay system cannot see the stage or the story graph.** It receives
   `GameplayContext { dt_ms, inputs, &mut Stats }` and nothing else.
3. **The renderer cannot see game concepts.** It consumes a `Frame` — quads and
   text runs — so layout is testable without a GPU.

Read [`docs/architecture.md`](docs/architecture.md) for the full picture and
[`docs/adr/`](docs/adr/) for why each decision went the way it did.

## Building

Requires Rust 1.85+ (edition 2024) and, on Windows, the MSVC toolchain.

```console
cargo check --workspace     # fastest feedback
cargo build                 # first run pulls the wgpu tree; minutes
cargo test --workspace      # 69 tests; render tests skip without a GPU
cargo run -p rustory -- probe
```

The dev profile builds dependencies at `opt-level = 3` — an unoptimized
`wgpu`/`naga` build is slow enough that a frame-rate problem looks like a
renderer bug — while workspace crates stay at `opt-level = 1`.

## Writing a story

```ron
Story(
    start: "prologue",
    nodes: {
        "prologue": (
            beats: [
                SetBackground(image: Some("bg_classroom.png")),
                Say(speaker: None, text: "The bell rings."),
                ShowActor(actor: "yuki", pose: "yuki_smile", slot: Left),
                Say(speaker: Some("yuki"), text: "You are still here?"),
            ],
            exit: Choose(
                prompt: Some("How do you answer?"),
                choices: [
                    (label: "Honestly", to: "honest"),
                    (label: "Change the subject", to: "deflect"),
                ],
            ),
        ),
    },
)
```

See [`stories/demo.ron`](stories/demo.ron) for a complete example, including a
gameplay handover. The demo text is Chinese, which also exercises CJK shaping and
line breaking.

## Adding gameplay

See [`docs/gameplay-integration.md`](docs/gameplay-integration.md). Short
version: new crate, implement `GameplaySystem`, register one line, reference it
from a script.

## Contributing

Read [`AGENTS.md`](AGENTS.md) first — it is the operating standard for this
repository and is written for both humans and coding agents.

Commits follow Conventional Commits in English, one logical change per commit.

## License

MIT OR Apache-2.0, at your option.
