# Gameplay Integration Guide

This guide explains **how to add a gameplay system**, and why none of those steps
touch the narrative core.

The short version: create a crate, implement `GameplaySystem`, add one
registration line in `driver.rs`, and reference it from a script with a
`Gameplay` exit. **Not one line of `rustory-core` changes.**

---

## 1. What the contract looks like

```rust
pub trait GameplaySystem {
    fn id(&self) -> SystemId;
    fn begin(&mut self, config: Option<&ron::Value>) -> Result<(), CoreError>;
    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey>;
    fn cancel(&mut self);
    fn prompt(&self) -> Option<String> { None }

    // Optional: only a system with state begin cannot rebuild needs these.
    fn save(&self) -> Result<Option<ron::Value>, CoreError> { Ok(None) }
    fn restore(&mut self, state: Option<&ron::Value>) -> Result<(), CoreError> { … }
}
```

Five methods with distinct jobs:

- `begin` takes the config the script wrote and parses it into a strong type
  **itself**. A failure returns an error and the handover does not happen.
- `step` advances one frame. Returning `Some(key)` means "finished, hand control
  back".
- `cancel` fires when the story is interrupted (window closed, a save loaded
  elsewhere). It must leave no residue.
- `prompt` is what the player sees. The default shows nothing.
- `save` / `restore` persist whatever `begin` cannot rebuild. Both have defaults,
  so a stateless system writes nothing. See section 6.

This is **everything** `step` can reach:

```rust
pub struct GameplayContext<'a> {
    pub dt_ms: u32,
    pub inputs: &'a [GameplayInput],   // Confirm | Select { index }
    pub stats: &'a mut Stats,          // BTreeMap<String, i32>
}
```

The stage, the story graph, the renderer, the window — none of them are in there.
That is not a convention. It is type-level isolation.

---

## 2. Five steps

### Step 1: create the crate

```
crates/rustory-gameplay-<name>/
├── Cargo.toml
└── src/lib.rs
```

```toml
[package]
name = "rustory-gameplay-<name>"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "RuStory gameplay system: <one line>"

[dependencies]
rustory-core = { workspace = true }
serde = { workspace = true }
ron = { workspace = true }
thiserror = { workspace = true }
```

Add one line to `members` and one to `[workspace.dependencies]` in the root
`Cargo.toml`.

### Step 2: define the config

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct MyConfig {
    pub rounds: u32,
    #[serde(default)]
    pub prompt: Option<String>,
}

impl MyConfig {
    pub fn from_value(value: Option<&ron::Value>) -> Result<Self, String> {
        let value = value.ok_or("config is required")?;
        let config: Self = value
            .clone()
            .into_rust::<Self>()
            .map_err(|err| err.to_string())?;
        if config.rounds == 0 {
            return Err("rounds must be positive".into());
        }
        Ok(config)
    }
}
```

**Validation belongs here.** A bad config should be rejected at handover, not
halfway through play.

### Step 3: implement the trait

```rust
pub struct MySystem {
    config: Option<MyConfig>,
    remaining: u32,
}

impl GameplaySystem for MySystem {
    fn id(&self) -> SystemId {
        SystemId::from("my.system")
    }

    fn begin(&mut self, config: Option<&ron::Value>) -> Result<(), CoreError> {
        // Validate first, then mutate: a rejected config must not leave the
        // system half-initialised.
        let parsed = MyConfig::from_value(config).map_err(|message| CoreError::GameplayFailed {
            system: "my.system".to_owned(),
            message,
        })?;
        self.remaining = parsed.rounds;
        self.config = Some(parsed);
        Ok(())
    }

    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        // Guard: `step` may be called before `begin` completed, and there is
        // nothing to do until it has.
        self.config.as_ref()?;

        // Input is read before the clock advances: an answer given on the frame
        // the timer expires still counts.
        for input in ctx.inputs {
            if let GameplayInput::Confirm = input {
                ctx.stats.add("my.system.confirms", 1);
                return Some(OutcomeKey::from("confirmed"));
            }
        }

        self.remaining = self.remaining.saturating_sub(ctx.dt_ms);
        if self.remaining == 0 {
            return Some(OutcomeKey::from("expired"));
        }
        None
    }

    fn cancel(&mut self) {
        self.config = None;
        self.remaining = 0;
    }

    fn prompt(&self) -> Option<String> {
        self.config.as_ref()?;
        Some(format!("{}ms left", self.remaining))
    }
}
```

### Step 4: register it (one line)

In `apps/rustory/src/driver.rs`:

```rust
pub fn default_registry() -> GameplayRegistry {
    let mut registry = GameplayRegistry::new();
    registry.register(Box::new(TimedChoice::new()));
    registry.register(Box::new(MySystem::new()));   // <- this line
    registry
}
```

### Step 5: use it from a script

```ron
"my_scene": (
    beats: [ Say(text: "Something is about to happen.") ],
    exit: Gameplay(spec: (
        system: "my.system",
        config: Some((rounds: 3000, prompt: Some("Press confirm"))),
        outcomes: {
            "confirmed": "you_did_it",
            "expired": "too_late",
        },
    )),
),
```

Note that `outcomes` lives in the **script**. The same system can have entirely
different consequences in different scenes without a code change.

---

## 3. Why this does not touch the core

| Concern | Owner | Does the core know? |
|---|---|---|
| Config fields | The system's own `MyConfig` | No — it forwards an opaque value |
| What an outcome means | The script's `outcomes` table | No — it treats it as a string key |
| Internal state | The system's own fields | No |
| When it is finished | The system returning `Some(key)` | Only that it *is* finished |

The only thing the core gained is: **if this system is registered, it can run.**
That is what "additive extension" means in practice.

---

## 4. Rules that matter

### Do not try to reach more state

`GameplayContext` has no stage and no story graph. Do not add a back door —
stuffing a serialised stage into `Stats`, for example.

If you genuinely need it, the correct sequence is:

1. Write an ADR explaining why this system must read the stage.
2. Add a **read-only** snapshot type, not `&mut Narrative`.
3. Update this document.

### `cancel` must actually clean up

Stories get interrupted: the window closes, a save reloads to a different node,
a developer hot-reloads. Residue left by `cancel` becomes dirty state inherited by
the next `begin`, and that class of bug is very hard to trace.

### Read input before advancing the clock

A player who presses confirm on the same frame the timer hits zero succeeded.
The reverse makes players feel they were robbed.

### Namespace any value written to `Stats`

```rust
ctx.stats.add("timed_choice.correct", 1);   // good
ctx.stats.add("correct", 1);                // will collide with another system
```

`Stats` keys are strings. The compiler will not catch a typo.

### Do not do heavy work in `step`

It runs every frame. Parse config in `begin`, allocate in `begin`; `step` does
arithmetic and state transitions only.

---

## 5. Writing tests

Testing a gameplay system **needs no story, no window and no GPU**. Build the
context directly:

```rust
#[test]
fn confirms_before_the_deadline() {
    let mut system = MySystem::new();
    system.begin(Some(&config)).expect("valid config");
    let mut stats = Stats::default();

    let outcome = system.step(&mut GameplayContext {
        dt_ms: 16,
        inputs: &[GameplayInput::Confirm],
        stats: &mut stats,
    });

    assert_eq!(outcome, Some(OutcomeKey::from("confirmed")));
}
```

Worth covering:

- every outcome key at least once;
- boundaries: the expiry frame, an impossible index, an empty config;
- `begin` works again after `cancel`;
- a rejected config leaves internal state **unchanged**.

See the nine tests in `crates/rustory-gameplay-timer/src/lib.rs`.

---

## 6. Saving and restoring state

### Do you need it?

Ask one question: **can `begin` reproduce this system's progress from the script
alone?**

- A countdown derived from an `elapsed_ms` counter in `Stats`: the answer is
  arguably yes, but even here the elapsed time is not in `Stats` unless you put it
  there. The timer crate implements `save` for exactly this reason — reading a save
  must not restart the clock.
- A quiz holding a list of answers and a cursor into the rounds: no. `Stats` holds
  integers, and inventing an encoding on top of them is the dynamic improvisation
  this project rejects.
- A shuffled deck, a generated board, a seeded random run: no.

If the answer is no, implement both methods. If it is yes, the defaults are correct
and you write nothing.

### The rule that gets people

> If `begin` cannot reproduce it, it belongs in `save`.

Getting this wrong does not fail loudly: the save simply resumes with a fresh
system, and the player loses invisible progress. That silence is why the rule is
stated twice — here and in the trait's doc comment.

### What to store, and what not to

Store only what changes during play. **Do not store the config**: it comes back
from the script through `begin`, and duplicating it lets a save carry a stale
duration that disagrees with the script it is loaded against.

Implementation, using the quiz as the reference:

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct QuizState {
    current: usize,
    answers: Vec<usize>,
}

fn save(&self) -> Result<Option<ron::Value>, CoreError> {
    if self.config.is_none() {
        return Ok(None); // never begun: nothing worth restoring
    }
    let state = QuizState {
        current: self.current,
        answers: self.answers.clone(),
    };
    encode_state(&state)
        .map(Some)
        .map_err(|message| CoreError::GameplaySaveFailed {
            system: SYSTEM_ID.to_owned(),
            message,
        })
}

fn restore(&mut self, state: Option<&ron::Value>) -> Result<(), CoreError> {
    let Some(value) = state else {
        return Ok(()); // no state: `begin` already reset to the start
    };
    let state: QuizState = decode_state(value).map_err(|message| {
        CoreError::GameplayRestoreFailed {
            system: SYSTEM_ID.to_owned(),
            message,
        }
    })?;

    // Validate against the config that `begin` just parsed. A save whose round
    // index runs past the config means the script was edited.
    let count = self.config.as_ref().map_or(0, |c| c.rounds.len());
    if state.current > count || state.answers.len() != state.current {
        return Err(CoreError::GameplayRestoreFailed {
            system: SYSTEM_ID.to_owned(),
            message: "saved progress does not fit this quiz".to_owned(),
        });
    }

    self.current = state.current;
    self.answers = state.answers;
    Ok(())
}
```

Two helpers save you from a `ron` quirk: `encode_state` and `decode_state` in
`rustory_core`. `ron` 0.11 has no `to_value`, so the encode path goes through text;
the helper keeps that detail in one place.

### Order on load

`begin` runs **first**, then `restore`. So `restore` can assume the config is
parsed and invariant state exists; it only overwrites what changed during play. It
also means a `restore` that fails leaves a cleanly begun system rather than a
half-mixed one.

### Refuse, do not discard

The default `restore` **rejects** state it cannot use:

```rust
fn restore(&mut self, state: Option<&ron::Value>) -> Result<(), CoreError> {
    match state {
        None => Ok(()),
        Some(_) => Err(CoreError::GameplayRestoreFailed { /* … */ }),
    }
}
```

If a save carries state for a system that never learned to restore it, the save
and the code disagree. Resuming while silently dropping that state would lose
invisible progress; refusing tells the player something is wrong.

### Testing it

The pattern that matters: finish half the gameplay, save, restore into a fresh
system, finish the rest, and assert the verdict matches an uninterrupted run.

```rust
#[test]
fn a_half_finished_run_resumes_at_the_same_round() {
    let mut quiz = Quiz::new();
    quiz.begin(Some(&config)).expect("valid");
    let mut stats = Stats::default();
    answer(&mut quiz, 0, &mut stats);

    let saved = quiz.save().expect("saves").expect("stateful");

    let mut resumed = Quiz::new();
    resumed.begin(Some(&config)).expect("valid");
    assert_eq!(resumed.current_round(), 0, "begin alone starts over");
    resumed.restore(Some(&saved)).expect("restores");

    assert_eq!(resumed.current_round(), 1);
}
```

Also cover: a save that no longer fits the config is rejected, and `cancel`
followed by `begin` starts clean.

## 7. Anti-patterns

```rust
// Wrong: the core now has to know about this system.
pub enum GameplaySpec {
    MySystemThing { rounds: u32 },
}
```

```rust
// Wrong: dynamically shaped, correctness only known at runtime.
ctx.stats_raw.insert("anything", Box::new(vec![1, 2, 3]));
```

```rust
// Wrong: letting gameplay mutate the narrative directly.
fn step(&mut self, narrative: &mut Narrative) { ... }
```

```rust
// Wrong: parsing config inside step (once per frame).
fn step(&mut self, ctx: &mut GameplayContext) {
    let config: MyConfig = ron::from_value(self.raw.clone()).unwrap(); // every frame!
}
```

The first three break architectural promises. The fourth is merely slow — but
when it is slow enough to drop frames, you will assume the renderer is at fault.

---

## 8. Working references

Two systems ship with the engine, chosen to bracket the range of the contract.

### `crates/rustory-gameplay-timer` — the simple end

- Config: `TimerConfig { prompt, duration_ms, options, correct }`
- Outcomes: `correct` / `wrong` / `timeout`
- Behaviours covered: input beats the clock, impossible indices are ignored,
  config is validated up front, `cancel` clears everything.
- Persists `remaining_ms` and the chosen index, so a save does not restart the
  clock.

### `crates/rustory-gameplay-quiz` — the stateful end

- Config: `QuizConfig { prompt, rounds: [{ question, options, correct }] }`
- Outcomes: `perfect` / `passed` / `failed`, with "passed" being half or more
- Persists a cursor and a growing list of answers: state no counter could hold.
- Demonstrates the load-time checks: a saved round index beyond the config is
  rejected rather than clamped, because resuming into a round that does not exist
  is not a recoverable situation.

Both appear in `stories/demo.ron` on a single path: the timed choice in the
`honest` branch, the quiz in `confession`. Neither system knows the other exists,
and the core mentions neither.
