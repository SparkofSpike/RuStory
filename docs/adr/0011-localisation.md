# ADR 0011 — Text a player reads has a name, and the engine resolves it

- **Status**: Accepted, implemented
- **Date**: 2026-10-27

---

## Context

Every line in a script was a `String`. For a project that ships in one language that is
correct and costs nothing; for a project that ships in three it is a wall, because the only
way to translate was to keep three copies of the whole story and hope they stayed
structurally identical.

Visual novels cross borders as a matter of course. A text-heavy engine that cannot be
translated is an engine for one market, and the decision is expensive to defer: it changes the
shape of every story file, the operation layer, the editor and the save format.

## Options

### A. The engine stays out of it

A project keeps `stories/en/`, `stories/ja/` and chooses a directory at launch.

Zero engine work. The cost is that a story becomes several files that must agree about
structure — add a node to one and the others silently diverge until a scene is missing in a
language nobody on the team reads. And a scene cannot mix languages, which is exactly what a
character who speaks two languages needs.

### B. The engine provides keys and tables

`Text` is either a literal string or a key; the story carries one table per language.

One graph of nodes, one set of branches, N tables. Structure is translated once because it
exists once. A save is a node and a beat index — neither is textual — so it survives a
language change.

The cost is real: a format change touching every place that reads text, and an indirection
that single-language projects must be able to ignore completely.

### C. Two fields: `text` plus an optional `text_key`

Tempting because it looks additive, and rejected for that reason. Two fields that describe the
same thing can disagree, and every reader then has to decide which wins. The bug would be a
line rendering in English while its key resolves to Japanese, and it would be nobody's fault.

## Decision

**B**, with one detail that makes the cost fall only on projects that want the feature:

```rust
#[serde(untagged)]
pub enum Text {
    Literal(String),
    Key { key: String },
}
```

The literal form is untagged, so `text: "The bell rings."` still parses. A single-language
script is unchanged, byte for byte, and the type costs it nothing — no table, no
`default_language`, no resolution step. A translated script writes `text: (key: "…")`.

`Beat::Say.text`, `Choice.label` and `Exit::Choose.prompt` all became `Text`. Labels and prompts
were included because an engine that translates dialogue and leaves the menu in English is not
translated.

## Resolution happens once, at load

`Story::localised(language)` returns a story whose every key is a literal. That is the only
place a key becomes a string, which is why the state machine, the renderer, the audio layer and
the save format never learned that translations exist.

`Narrative::new` calls it automatically when the story has keys and an obvious language, so a
caller cannot forget. A caller that wants a specific language resolves first and the step
becomes a no-op.

Two things are load errors rather than runtime surprises:

- A key no language defines (`CoreError::UndefinedKey`) — a typo.
- A key missing from the language being played (`CoreError::MissingTranslation`, naming both).

A player staring at an empty dialogue box cannot tell a missing translation from a broken
renderer. Neither can whoever they report it to.

## The bug this exposed: saves were keyed on text

`Story::checksum` hashed the RON form of the whole story. That was wrong before this change and
merely invisible: it meant that fixing a typo in a line invalidated every save of that story.

Translation made it fatal — the English and Japanese versions of one story have different text,
so a save made in English could not be loaded after switching language, which is precisely the
feature.

The fingerprint now covers **shape**: the start node, each node's id, the number and kind of its
beats, and where its branches lead. Not a word of dialogue.

- Rewriting a line, or changing language: fingerprint unchanged, saves stay valid.
- Inserting a beat, deleting a node, repointing a branch: fingerprint changes, because a saved
  index would then point somewhere else.

That is the property a save actually needs — that an index still means the same thing — and it
is what the old comment claimed it was doing.

## What this deliberately does not solve

- **No plural rules, no gendered forms, no ICU.** A key resolves to one string. Languages that
  need the caller to know a count before choosing a form will need the engine to pass one; the
  honest position is that this design covers "the same lines in another language" and not
  "grammatically generated language", and the first project that needs the latter should get an
  ADR of its own rather than a special case bolted on.
- **No fallback chain.** A missing translation is an error, not a silent fall back to English.
  A silent fallback is how a release ships with half a language and nobody notices.
- **No tooling for the tables.** `rustory check` reports which language is short and by which
  keys; extracting strings to a translation service is a project's problem.

## Verification

- `crates/rustory-vn-core/tests/localisation.rs` — eleven tests: a story without keys plays as
  written; a keyed story with two languages and no default refuses to play; the default is used
  without being asked; localising replaces keys and keeps literals; a missing translation names
  the key and the language; an undefined key fails at load; labels and prompts translate; a save
  restored in another language lands on the same line, in the new language.
- `crates/rustory-vn-script/tests/script.rs` — six tests on the format, including that the
  shipped demo still parses unchanged.
- `stories/localised.ron` — one story in English, Japanese and Simplified Chinese. Verified by
  running all three: `rustory transcript stories/localised.ron --language ja` prints Japanese,
  `zh-Hans` prints Chinese, and `rustory shot --language …` produces three different frames
  (`shots/localised-{en,ja,zh}.png`).
- `rustory check` reports the language list, the default, the number of keys, and per-language
  completeness; `--json` carries the same as `languages`, `default_language`, `text_keys` and
  `translations`.
