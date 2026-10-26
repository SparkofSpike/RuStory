# Architecture

This document describes RuStory's layers, its data flow, and the invariants that
break the design if violated. The **reasoning** behind each choice lives in
[`adr/`](adr/); this page describes only the shape things have right now.

---

## 1. Structure in one line

```
script (.ron) ──▶ Story ──▶ Narrative ──▶ Frame ──▶ wgpu ──▶ pixels
                                │
                                └──▶ GameplaySystem (pluggable)
```

The narrative layer is the only owner of game state. Neither the renderer nor a
gameplay system holds any.

---

## 2. Four crates and one dependency direction

```
        apps/rustory
        ├── rustory-render
        ├── rustory-script
        ├── rustory-gameplay-timer
        ├── rustory-gameplay-quiz
        └── rustory-core
                ▲
   rustory-render ─┘        (depends on core's types; core does not depend on it)
   rustory-script ─┘
   rustory-gameplay-timer ─┘
   rustory-gameplay-quiz ──┘
```

**Every dependency points at `rustory-core`.** The core depends on nothing above
it. Cargo's dependency declarations enforce this, not discipline.

| Crate | Owns | Must not do |
|---|---|---|
| `rustory-core` | Story model, state machine, gameplay contract | Render, open windows, touch the filesystem |
| `rustory-script` | RON parsing, structural validation | Play, advance state |
| `rustory-render` | Draw list → pixels | Know what a story, actor or choice is |
| `rustory-gameplay-timer` | One concrete gameplay system | Read the stage or the story graph |
| `rustory-gameplay-quiz` | A second one, with state no counter could hold | Ditto |

---

## 3. Data flow

### 3.1 One step forward

```
player clicks
  → the app translates it into an intent (advance / choose / select)
  → Narrative mutates state and produces Vec<NarrativeEvent>
  → the app rebuilds Frame from scratch (no incremental mirror)
  → Renderer draws the Frame
```

The renderer **caches no state**. It re-reads `Narrative` every frame. That is
deliberate: one less mirror to keep in sync is one less class of "the picture
disagrees with the state" bug.

### 3.2 Gameplay handover

```
node exit: Gameplay(spec)
  → Narrative.mode = Suspended { spec }
  → the app calls Narrative.step_gameplay(registry, dt, inputs) each frame
      → first call runs begin(config); the system parses its own config
      → later calls run step(ctx); the system returns Option<OutcomeKey>
  → the system returns Some(key)
      → Narrative looks up spec.outcomes[key] for the next node
      → Narrative.mode returns to playback
```

The load-bearing detail: **the core never parses the config and never
interprets the key.**

### 3.3 Saving and loading

```
Narrative::save(&registry)
  → copies cursor, beat index, mode, reveal count, stage, stats
  → if suspended, asks that one system for its state (opaque ron::Value)
  → SaveState { version, story_checksum, … }

Narrative::from_save(story, &save, &mut registry)
  → refuses on version, checksum, node, beat index or mode/exit mismatch
  → clamps the reveal count to the current line
  → if suspended: begin(config) then restore(state)
```

Loading produces **no events**: it is a restore, not a traversal, and a renderer
replaying node-entry events would redraw a scene the player is already looking at.

---

## 4. Invariants

Each of these, if broken, breaks a promise the architecture makes. Write an ADR
before changing one.

1. **`rustory-core` depends on no rendering or window code.**
   Break this and the core can no longer be tested without a GPU, which raises
   the cost of every test by an order of magnitude.

2. **A gameplay system cannot reach the stage, the story graph or the renderer.**
   It receives `GameplayContext { dt_ms, inputs, &mut Stats }` and nothing else.
   Need more? Change the contract, and write the ADR first.

3. **Adding a gameplay system does not modify `rustory-core`.**
   If it does, the contract is wrong. Fix the contract.

4. **Referential integrity is checked at load time, not at use time.**
   Dangling gotos, unmapped outcome keys and goto cycles must fail before the
   first frame.

5. **The renderer knows no game concepts.**
   `Story`, `Actor` or `Choice` appearing in `rustory-render` is a leak. It knows
   `Frame`, `Quad` and `TextItem`.

6. **Typewriter text advances by grapheme cluster, never by `char`.**
   `char` splits emoji and combining marks into pieces.

---

## 5. Module map

```
crates/rustory-core/src/
  ids.rs          NodeId / ActorId / SystemId / OutcomeKey (newtypes, so they cannot be mixed up)
  story.rs        Story / Node / Beat / Exit / Choice / GameplaySpec
  stage.rs        Stage (background + actors on screen); explicit fields, no dynamic component table
  narrative.rs    State machine: cursor, beat index, reveal progress, Mode
  event.rs        NarrativeEvent, TickReport
  gameplay.rs     GameplaySystem trait, GameplayContext, Stats, Registry
  error.rs        CoreError

crates/rustory-script/src/lib.rs
  parse_story / load_story; errors carry line and column

crates/rustory-render/src/
  frame.rs        Frame / Quad / TextItem / Rect / Rgba / Uv
  image.rs        Rgba8Image (PNG read and write, solid fills, gradients)
  renderer.rs     wgpu pipeline, texture upload, offscreen readback

crates/rustory-gameplay-timer/src/lib.rs
  Timed choice: TimerConfig, TimedChoice

apps/rustory/src/
  main.rs         CLI dispatch: probe / check / transcript / gen-assets / shot / play
  driver.rs       Click semantics, registry, texture loading, transcript
  scene.rs        The only translation layer from Narrative to Frame
  play.rs         winit event loop, surface, input mapping
  assets.rs       Procedural placeholder art
```

---

## 6. Why there is no general-purpose ECS

The project originally planned to build on Bevy ECS, on the reasoning that
"gameplay = an independent System". Implemented for real, the analogy does not
hold:

- A visual novel's state shape is **fixed and small**: one stage (a background
  plus a few actors), one cursor, one set of counters.
- ECS pays off with "many heterogeneous entities in unknown combinations". A
  visual novel has neither.
- The costs are real: entity remapping complicates saves (see ADR 0006), and
  `Query`'s implicit access sets make "what can a gameplay system touch?" stop
  being enumerable.

So the current approach is **explicit struct fields for `Stage`, a narrow map for
`Stats`, trait objects for gameplay.** All three are enumerable, serialisable and
testable.

If a genuine "thousands of homogeneous entities needing batch processing" system
appears later — a bullet-hell minigame, say — give *that* system its own storage.
Do not wrap the whole engine in an ECS for it.

---

## 7. Testing strategy

| Layer | How it is tested | Needs a GPU? |
|---|---|---|
| State machine | Integration tests, pure memory | No |
| Story format | Positive cases plus failures (dangling refs, syntax errors) | No |
| Gameplay systems | Build a `GameplayContext` and step it | No |
| Layout | Assert rectangle coordinates inside a `Frame` | No |
| Rendering | Offscreen render → read pixels back → compare relative luminance | **Yes** |
| Window | Run it and check for diagnostics | Yes |

Render tests **skip** rather than fail on a machine with no adapter: a missing
GPU is an environment fact, not a regression.

---

## 8. Known gaps

- **No rollback history.** `SaveState` supports it, but nothing keeps a ring of
  past snapshots yet — that needs UI for picking a point, not just storage.
- **No audio.**
- **No channel for gameplay to read narrative state.** A system cannot tell which
  dialogue round it is in; it has to be passed that through config. When this is
  needed, add a read-only snapshot — do not open up `Narrative`.
- **`--advance` is unintuitive.** It counts "one interaction", and one
  interaction can span two states (finishing a line, then advancing a beat).
  Inside a gameplay system it plays the whole thing to completion, always choosing
  the first option.
- **A bad gameplay config surfaces only when playback reaches that node**, not at
  load time.
- **No built-in typeface.** The engine ships no fonts; a project supplies its own
  through `assets/fonts/` or `--font`. See ADR 0007. The demo therefore renders in
  whatever the host considers its default sans-serif, which is correct but means
  screenshots differ between machines.
