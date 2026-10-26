# ADR 0007 — Fonts are the project's responsibility, not the engine's

- **Status**: Accepted, implemented
- **Date**: 2026-10-27

---

## Context

An earlier note in `architecture.md` listed "fonts come from the host system" as a
known gap, on the reasoning that a player without the font sees replacement boxes.
That framing was wrong about what this software is.

RuStory is an engine. It is not a game, and not an authoring tool bundled with
assets. The person shipping a game decides what it looks like, which includes the
typeface — and they have to, because:

- A typeface is a design decision, not a platform default. Nobody wants an engine
  picking their body text.
- Fonts are licensed. Shipping one inside an engine forces the engine's licence onto
  every project that uses it, and many commercial faces cannot be redistributed.
- Font files are large. A CJK face is 10MB+ uncompressed; multiplying that by every
  project that never needed it is a waste decided on their behalf.

The real question was never "does the player have a font". It was: **can a project
supply one, and can a creator tell whether it took effect?**

## Options

### A. Bundle a font and make it the default

- Engine controls the look, but takes on licensing, size, and the presumption of
  knowing better.

### B. Depend entirely on system fonts

- Smallest engine. But then a project that *does* have a specific look in mind has
  no supported way to get it, and "the font did not apply" becomes unanswerable.

### C. Provide loading and selection; ship nothing

- The engine gains a seam where a project hands it typefaces, and reports clearly
  when a name does not resolve.
- No font, no licence, no size in the engine.

## Decision

**Adopt C.**

## What the engine provides

```
Renderer::load_font(path)          -> Vec<String>   // families the file declares
Renderer::load_fonts_dir(dir)      -> Vec<String>   // skips junk, keeps going
Renderer::set_sans_serif_family(f) -> bool          // false if the name is unknown
Renderer::has_family(name)         -> bool
Renderer::families()               -> Vec<String>   // diagnostics
```

And at the layout level, `TextItem::with_font(FontChoice)`, where `FontChoice` is
`SansSerif` (default) / `Serif` / `Monospace` / `Named(String)`.

The application layer adds one convenience: `<assets>/fonts/` is scanned
automatically, and the first family found becomes the default unless `--font`
names another. That is the whole "just drop a file in" path — a creator does not
have to learn a flag or discover the internal name of their own file.

## Why the family names come back from the loader

`load_font` returns the families the file *declares*, not the filename. A creator
who receives `SourceHanSansSC-Regular.otf` has no way to know the shaper wants
`Source Han Sans SC`; the loader tells them.

This is not hypothetical: the test that exercises the path loads a system file named
`AGENCYR.TTF` and gets back `Agency FB`.

## What the engine deliberately does not do

- **Bundle any font.** Not even for the demo. The demo currently renders in the
  platform's default sans-serif, and that is correct — a demo that shipped a
  typeface would make the engine's install larger for everyone and blur the line
  between engine and game.
- **Subset fonts.** Useful for shipping, irrelevant to an engine.
- **Detect missing glyphs and warn.** That is a rendering-quality feature for the
  game to build, not a correctness problem for the engine to police.
- **Choose a fallback chain.** `cosmic-text` already does this per character, and
  reimplementing it would be exactly the kind of wheel ADR 0001 says not to build.

## Consequences

**Gained**

- No font licensing in the engine, no size, no opinion about typography.
- A project can load any typeface it has the rights to use.
- A wrong family name fails loudly with an actionable message instead of rendering
  in a silent substitute.

**Paid**

- A game that wants consistent typography across machines must ship the font
  itself. That is the intended division of labour, not a defect.
- The demo looks like whatever the host's default sans-serif is, so screenshots
  differ between machines. Acceptable: screenshots document behaviour, not design.

## Verification

- `crates/rustory-render/tests/fonts.rs` — 8 tests: loading a real font and
  recovering its declared families, a missing file, a file that is not a font, an
  unknown family name, selecting an installed family, a directory with junk in it,
  and the two `FontChoice` defaults.
- End to end: placing a TTF in `assets/fonts/` and running `rustory shot` renders in
  that face and prints which family it chose; `--font <name>` overrides it;
  `--font <bogus>` exits non-zero with a message naming the flag.
