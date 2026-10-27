# ADR 0009 — Sound is a beat; a one-shot is an event, music is state

- **Status**: Accepted, implemented
- **Date**: 2026-10-27

---

## Context

The engine had no audio, and `architecture.md` said so. A visual novel without sound is
not obviously broken — plenty are read in silence — but the *engine* cannot be the reason a
project has no soundtrack. Three questions had to be answered before a line of code:

1. **Where does an audio instruction live?** In a node's exit, in a beat, or nowhere (the
   host infers it from what changed)?
2. **What is remembered?** If the player saves mid-scene and resumes, does the music come
   back?
3. **Which crate owns it?** `rustory-vn-core` must run with no device, no window and no GPU,
   or its tests stop being usable on a build machine.

The third question constrained the first two: anything the core has to *do* with audio must
be expressible without opening a device.

## Options

### A. Audio as a property of the node

`Node { music: Option<String>, ... }`. Simple to validate, and the track changes exactly
when the scene changes.

But music is not scene-shaped. A confrontation that starts quietly and drops into a
different theme halfway through a paragraph is ordinary direction, and this design cannot
express it without splitting the scene into two nodes for no narrative reason.

### B. Audio as a beat

`Beat::Music { track, fade_ms }` and `Beat::Sound { clip, volume }`, sitting in the same
ordered list as `Say` and `ShowActor`.

The author writes the scene in the order it happens, which is the reason beats exist at all
(see the note at the top of `story.rs`). A track can change between two lines, or in the
middle of a paragraph, and the cursor model does not need a second dimension.

The cost is real: every `match` over `Beat` had to grow two arms. Nine places. That is the
price of a data model that is honest about the timeline, and it is paid once.

### C. The host infers audio from what changed

`BackgroundChanged { image: Some("classroom.png") }` → the host plays `classroom.ogg`.

No new types, no script syntax, no ADR. It is also implicit magic: the author cannot say
"this room is silent", cannot pick a track that is not named after the background, and
cannot explain to the host why a door slammed. Explicit beats win.

## Decision

**B.** Audio instructions are beats.

- `Beat::Music { track: Option<String>, fade_ms: Option<u32> }` — `None` stops the music.
  Non-blocking: the story keeps moving while the fade runs.
- `Beat::Sound { clip: String, volume: Option<f32> }` — a one-shot.

The narrative reports both as events (`MusicChanged`, `SoundPlayed`), and carries the
current track as **state** (`Narrative::music`), which is copied into `SaveState`. That
asymmetry is the decision, not an accident:

- **Music is state.** It is still playing when nothing is happening, so a save that forgot
  it would resume a scene into silence.
- **A one-shot is an event.** It happened and stopped. Storing it would mean replaying a
  door slam every time the player loads a save inside the scene where the door is still
  open.

## The shape of a transition

One track at a time. Changing the music fades the current track out and then fades the new
one in — it does not run both simultaneously.

A parallel crossfade needs two live players, two gains, and a rule for what happens when a
third cue arrives while both are moving. A serial one is a single number travelling in a
straight line, and every state it can be in is visible in one enum. For a transition most
listeners describe as "smoother", the extra machinery does not pay for itself; if a project
ever needs a true crossfade, it can layer two `MusicDirector`s behind its own mixer, because
this crate owns no device.

A `fade_ms` of zero cuts. That is what a scene that changes track on a beat wants, and it is
also the path that needs no timer at all.

## Where the pieces live

- **`rustory-vn-core`** owns the beats, the events and the track-as-state. It never opens a
  device; its tests run anywhere.
- **`rustory-vn-audio`** owns the fade state machine (`MusicDirector`) and nothing else. No
  device, no decoding, no file paths. Every behaviour is a pure function of events and
  elapsed milliseconds, which is why it has nine tests that run in microseconds.
- **The host** (`apps/rustory-vn`) owns the device, the decoder and the file lookup. It is the
  only place that knows `rodio` exists.

That layering is the same rule as ADR 0001: the engine decides, the platform layer carries
it out. It also means swapping `rodio` for `kira`, or for a mixer in a browser, touches one
module.

## Files and names

A track name is a file stem, exactly like a background image: `assets/audio/<name>.<ext>`,
with `ogg`, `wav`, `flac` and `mp3` accepted. When a stem exists with several extensions the
choice is fixed by that list rather than by directory order — a scan that picks whatever the
filesystem yields first is a bug that only appears on someone else's machine.

A name that resolves to nothing is reported once per run and then ignored. An authoring typo
should be visible; a scene that cues the same missing track every visit must not print a
line every frame.

## When there is no device

The game runs silent. `Audio::unavailable` records why, `rustory play` prints it at startup,
and `rustory probe` reports the device. Refusing to start over a missing sound card would
trade a degraded game for no game.

## Verification

- `crates/rustory-vn-core/tests/narrative.rs` — a music cue sets the track, stopping is also a
  cue, a one-shot leaves no state behind, and neither blocks the player.
- `crates/rustory-vn-core/tests/save.rs` — the track survives save and load, and a save written
  before audio existed still loads (the field is `serde(default)`).
- `crates/rustory-vn-audio/src/lib.rs` — nine tests over the fade machine: immediate cues,
  fade-in timing, fade-out before a swap, re-cueing the same track doing nothing, a cue
  mid-fade retargeting, overshoot, and non-audio events being ignored.
- `apps/rustory-vn/src/assets.rs` — the placeholder sounds are generated as WAV by the same
  command as the placeholder art, so the demo is audible from a fresh clone with no binary
  in the repository.
- `rustory probe` reports the device and how many files were indexed.

## Consequences

- A story can be written with no audio at all; nothing is mandatory and no directory is
  required.
- Adding a third kind of audio (voice, ambience on a second track) is another beat variant
  and another arm in the same nine matches. If that becomes common, the right move is a
  general `Audio { layer, ... }` beat rather than a fourth parallel case — but not until a
  real story needs it.
- `MusicDirector` is deliberately one track. A project wanting layered ambience writes its
  own mixer; this crate makes no promise it cannot keep.
