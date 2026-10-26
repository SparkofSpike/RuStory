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

Two systems ship as evidence, chosen to bracket the range of the contract:

- `crates/rustory-gameplay-timer` — a countdown over a list of options.
- `crates/rustory-gameplay-quiz` — several rounds, a growing list of answers, and
  a verdict that only exists at the end.

`rustory-core` mentions neither. It has no notion of timers, options, rounds,
scores or correctness. Each crate parses its own config from an opaque value,
reports outcome keys, and lets the *script* decide where those keys lead. Each
persists its own state into a save through the same trait.

The entire integration is two lines:

```rust
registry.register(Box::new(TimedChoice::new()));
registry.register(Box::new(Quiz::new()));
```

The quiz is the stronger half of the argument. A countdown can almost be rebuilt
from a few shared counters, so a critic could say the contract only accommodates
simple things. A quiz cannot: it needs a cursor and a list, which is why it also
proves that a save can carry state the core has never heard of.

If adding a system ever requires editing `rustory-core`, the contract is wrong
and the contract gets fixed — not the core.

## Status

**Playable vertical slice.** Working today:

- Load a story from RON, with reference checking before the first frame
- Background, sprites and typewriter dialogue
- Click to advance; first click completes the line, second advances
- Branching choices
- Gameplay handover, with **two** systems: a timed choice and a multi-round quiz
- **Save and load**, including gameplay state mid-system (`F5` / `F9` in a window)
- `rustory play` in a window, or `rustory shot` to render a frame headlessly
- 116 tests, including offscreen GPU renders that assert where things land

Not yet: audio, rollback history, WASM plugins, an authoring tool, packaged fonts.
Those have ADRs describing the plan, not code pretending to be one.

## Try it

```console
$ cargo run -p rustory -- gen-assets assets       # placeholder art
$ cargo run -p rustory -- check stories/demo.ron  # parse + validate
$ cargo run -p rustory -- play stories/demo.ron   # play it
$ cargo run -p rustory -- shot stories/demo.ron --out frame.png --advance 4

$ cargo run -p rustory -- save stories/demo.ron --advance 6 --out save.ron
$ cargo run -p rustory -- resume stories/demo.ron save.ron --finish
```

The demo runs without a window too:

```console
$ cargo run -p rustory -- transcript stories/demo.ron
After school, the classroom holds only two people.
Yuki: You haven't left yet?
Kai: I was waiting for you.
...
```

## Screens

| Dialogue | Choice | Timed choice | Quiz |
|---|---|---|---|
| ![dialogue](shots/dialogue.png) | ![choice](shots/choice.png) | ![gameplay](shots/gameplay.png) | ![quiz](shots/quiz.png) |

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
| Gameplay | `rustory-gameplay-timer`, `rustory-gameplay-quiz` | Two systems bracketing the range: a countdown, and state no counter could hold |
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
cargo test --workspace      # 116 tests; render tests skip without a GPU
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
gameplay handover. Story text can be any language: CJK shaping and line breaking
are exercised by a dedicated renderer test that pushes a Chinese paragraph through
the same code path.

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
