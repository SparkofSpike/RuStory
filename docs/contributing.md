# Contributing

RuStory is an engine meant to be used by people who are not its author, so the rules here
are about keeping it usable rather than about style preferences.

## Before you write code

Read, in this order:

1. [`README.md`](../README.md) — what the project claims, and the commands that prove it.
2. [`docs/architecture.md`](architecture.md) — the layers and the dependency direction.
3. [`docs/adr/`](adr/) — every architectural decision, with the options that were rejected.
   An ADR outranks any summary, including this page.

If a change contradicts an ADR, say so in the pull request and propose a new ADR. Do not
quietly work around a decision: the next person will read the ADR and assume the code
follows it.

## Build and verify

```console
cargo check --workspace            # fastest feedback
cargo test --workspace             # core logic; GPU tests skip without an adapter
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

CI runs the same four and also executes every command in the README, so a documented
command that stops working fails the build rather than a reader's first attempt.

`profile.dev` puts dependencies at `opt-level = 3`. A low-optimised wgpu build is slow
enough that a frame-rate problem looks like a renderer bug. Do not change it.

## The four rules that are not negotiable

1. **The engine is ours; the platform layer is borrowed.** World storage, scheduling, the
   narrative state machine, the story format and the gameplay contract are written here.
   Window creation, GPU access, glyph rasterisation and audio output are borrowed from
   crates that already do them well. Bevy, Godot and macroquad are framework-sized: they
   bring their own ECS and main loop, which is the thing this project exists to own.
2. **Extension is addition.** A new gameplay is a new crate implementing `GameplaySystem`,
   registered in one place. If adding a gameplay requires editing `rustory-core`, the
   architecture is wrong — fix it before adding the feature.
3. **Boundaries are checkable.** Cross-system state is an `enum`, a `trait` or an event.
   `HashMap<String, Box<dyn Any>>` is not a design.
4. **No unverified claims.** Anything in `README.md` or `docs/` that says "supports X" needs
   a command or a test behind it. Planned work is written as planned.

## Commits

Conventional Commits, in English, one logical change per commit:

```
<type>(<scope>): <subject>
```

`type` is one of `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `perf`. `scope` is the
crate name or a domain word: `core`, `script`, `render`, `edit`, `gameplay`, `cli`, `ci`,
`docs`.

The body explains why the change exists and what it costs. "Updated the file" is not a
message; "the panel claimed typing produced one undo step per character, and it did" is.

Do not add `Co-Authored-By` trailers for tools. Authorship is the person who decided.

## Adding a gameplay

[`docs/gameplay-integration.md`](gameplay-integration.md) is the guide, with a minimal
example. The shape is always the same:

1. New crate, `#![forbid(unsafe_code)]`, depending on `rustory-core` and nothing above it.
2. A type implementing `GameplaySystem`: `step`, `prompt`, and `save`/`restore` if it holds
   state that a save must carry.
3. One registration line in the app's registry.
4. A story node whose exit is `Exit::Gameplay` naming the system and mapping each outcome
   key to a node.

Then add a test that drives the system with no window and no GPU. `rustory-gameplay-timer`
(small) and `rustory-gameplay-quiz` (stateful, saved) are the two references; two is the
minimum that makes "it is compositional" a fact rather than a hope.

## Changing the architecture

Write an ADR. `docs/adr/0000-template.md`-shaped: context, the options with their costs, the
decision, and the consequences. Number it after the highest existing one. Then update
`docs/architecture.md` and any code comment that described the old rule.

An ADR that lists only the option that was chosen is not an ADR.

## Reporting a problem

Include the command you ran, the output you got and the output you expected. If it is a
rendering issue, `rustory shot <story> --out frame.png` plus the story is reproducible;
a screenshot without the script is not.
