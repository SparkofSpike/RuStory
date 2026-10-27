# Changelog

Not a release history yet — the crates are unpublished (see [`docs/releasing.md`](docs/releasing.md)).
This file exists because "what changed for me" is a question `git log` answers badly, and because
a release checklist that starts from nothing is a checklist nobody follows.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning will follow
SemVer once there is a published version to be compatible with.

## [Unreleased]

### Added

- **Localisation.** Story text can be a literal or a key; a story carries per-language tables and
  a default language. A missing translation is a load error naming the key, not a blank line.
  `rustory transcript --language <tag>` and `rustory shot --language <tag>` play in one language.
- **`--allow-missing`** for `transcript` and `shot`: play a partly translated story using the
  default language's line where one is missing, and the key name when there is no default.
  `check` stays strict, so the holes are still reported.
- **A gameplay system can read where the story is.** `GameplayContext` and `begin` both receive an
  owned, read-only `NarrativeSnapshot`: node, beat position, every node visited and every choice
  made. History is stored in the save, so a resumed system sees what the original saw.
- **A gameplay system draws its own screen.** `GameplayView` replaces the single-string prompt:
  text, panels, images and bars in a 1280×720 design box, coloured by role. Both front-ends paint
  it with the same code.
- **`rustory shot --at <node>:exit`** — stop past a node's last beat, so a choice or a gameplay
  screen can be rendered without a window.
- **An editor timeline.** `Ctrl+H` lists every state the open tab can jump to, labelled with the
  operation that produced it. A jump is undo-or-redo repeated, so it cannot disagree with the
  keyboard shortcuts.
- **A WebAssembly tier.** `rustory-vn-gameplay-wasm` runs a module through the same
  `GameplaySystem` trait, with no imports at all and a per-call instruction budget. The guest
  returns an outcome key, an optional view and an optional save state, all as RON.
- **Plugins resolved by name.** A story names a module (`system: "doors"`); the module comes from
  `--plugin name=path`, the story's own `plugins` map, or `plugins/<name>.wasm` beside the story
  or one level up — in that order. `rustory check` reports which source supplied each one.
  `examples/wasm-guest` is a working 1.6 KB module written against the ABI by hand.
- **Pointer input.** `GameplayInput::Pointer { x, y, phase }` in the view's design box, with
  `Pressed` and `Released`. A button reads the press, a drag remembers it and decides on the
  release. No hover: nothing shipped needs it.
- **`rustory-editor`'s history panel** on `Ctrl+H`: every state the open tab can jump to, labelled
  with the operation that produced it.
- **A save screen.** `F5` / `F9` open a 3×3 grid plus a quick slot, each card showing the story
  title, the node, the line the player was on and the language. Four states, each said in its own
  words: empty, occupied, damaged, and written for a different story version. `GridScreen` is the
  default behind a `SaveScreen` trait, so a project can replace it wholesale.
- **`rustory saves <story>`** lists the same slots from a terminal, `--json` for a program,
  `--delete <slot>` to remove one, and `--shot out.png --mode load` to render the screen without a
  window.

### Changed

- Packages are published as `rustory-vn-*`; the repository stays `RuStory` and the commands stay
  `rustory` and `rustory-editor`. See [ADR 0016](docs/adr/0016-three-names.md).
- `rustory-vn-editor`'s binary is `rustory-editor`. The package keeps its `apps/` name, as the
  player's package is `rustory-vn` while its binary is `rustory`.
- A save's integrity check covers the **shape** of the story, not its text: fixing a typo or
  switching language no longer invalidates a save.
- `GameplaySystem::prompt` is removed rather than deprecated. Every system returns a view or
  nothing, so no system can quietly stay on the old path.

### Fixed

- `--at <node>:beat` no longer mistakes an intermediate gameplay node for having left the target.
- `rustory check` now reports a gameplay config its system rejects, and exits non-zero. It used
  to pass a story whose config could not be parsed, because configs are opaque to the core.
- The editor's binary is `rustory-editor`, matching ADR 0016; it was still `rustory-vn-editor`.

## [0.1.0] — unreleased

The vertical slice: RON stories, reference checking, background and sprites, typewriter dialogue,
branching, two gameplay systems, save and load, a CLI and a GUI editor over one operation layer.
