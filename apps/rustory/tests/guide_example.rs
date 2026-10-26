//! Compile-check the example in `docs/gameplay-integration.md`.
//!
//! Documentation that shows code nobody ever compiled drifts silently. The guide
//! is the entry point for adding a gameplay system, so its example is the single
//! most important snippet in the repository to keep honest.
//!
//! This file is not a test of the engine; it is a test of the documentation. The
//! code below is transcribed from the guide with only the surrounding scaffolding
//! (imports, a test function) added.

use rustory_core::{
    CoreError, GameplayContext, GameplayInput, GameplaySystem, OutcomeKey, Stats, SystemId,
};
use serde::Deserialize;

// --- transcribed from docs/gameplay-integration.md, step 2 ---------------

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

// --- transcribed from docs/gameplay-integration.md, step 3 ---------------

pub struct MySystem {
    config: Option<MyConfig>,
    remaining: u32,
}

impl MySystem {
    // Kept as `new` rather than Default because the guide shows `MySystem::new()`
    // and this file must match the guide, not improve on it.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            config: None,
            remaining: 0,
        }
    }
}

impl GameplaySystem for MySystem {
    fn id(&self) -> SystemId {
        SystemId::from("my.system")
    }

    fn begin(&mut self, config: Option<&ron::Value>) -> Result<(), CoreError> {
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

#[test]
fn the_guide_example_compiles_and_behaves_as_documented() {
    let config: ron::Value = ron::from_str("(rounds: 100)").expect("config parses");

    let mut system = MySystem::new();
    system.begin(Some(&config)).expect("valid config");

    // Confirming wins over the clock, as the guide states.
    let mut stats = Stats::default();
    let outcome = system.step(&mut GameplayContext {
        dt_ms: 16,
        inputs: &[GameplayInput::Confirm],
        stats: &mut stats,
    });
    assert_eq!(outcome, Some(OutcomeKey::from("confirmed")));
    assert_eq!(stats.get("my.system.confirms"), 1);

    // And the clock path reaches its documented outcome.
    let mut system = MySystem::new();
    system.begin(Some(&config)).expect("valid config");
    let mut stats = Stats::default();
    let outcome = system.step(&mut GameplayContext {
        dt_ms: 100,
        inputs: &[],
        stats: &mut stats,
    });
    assert_eq!(outcome, Some(OutcomeKey::from("expired")));
}

#[test]
fn the_guide_example_rejects_a_bad_config_before_touching_state() {
    let bad: ron::Value = ron::from_str("(rounds: 0)").expect("parses");

    let mut system = MySystem::new();
    let err = system.begin(Some(&bad)).expect_err("must reject");
    assert!(
        matches!(err, CoreError::GameplayFailed { .. }),
        "unexpected error: {err}"
    );
}

// --- transcribed from docs/gameplay-integration.md, step 4 ---------------
//
// The registry line, checked against the real registry type.

#[test]
fn the_guide_registration_line_works() {
    let mut registry = rustory_core::GameplayRegistry::new();
    registry.register(Box::new(MySystem::new()));
    assert!(registry.contains(&SystemId::from("my.system")));
}
