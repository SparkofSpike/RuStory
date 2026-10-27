//! A timed choice: the reference implementation of a RuStory gameplay system.
//!
//! It exists to prove a specific claim — that a gameplay system can be added
//! without editing the narrative core. Nothing in `rustory-vn-core` mentions
//! timers, options or correctness; this crate owns all of that, and the core
//! only ever forwards an opaque config and receives back an outcome key.
//!
//! # Writing one in a script
//!
//! ```ron
//! (
//!     system: "timer.timed_choice",
//!     config: (
//!         prompt: "Pick the honest answer.",
//!         duration_ms: 3000,
//!         options: ["\"It was me.\"", "\"Not me.\""],
//!         correct: 0,
//!     ),
//!     outcomes: {
//!         "correct": "scene_won",
//!         "wrong": "scene_lost",
//!         "timeout": "scene_lost",
//!     },
//! )
//! ```
//!
//! The `outcomes` table lives in the *script*, so the same system can mean
//! different things in different scenes without a code change.

#![forbid(unsafe_code)]

use rustory_vn_core::prelude::*;
use serde::Deserialize;

/// The system id scripts use.
pub const SYSTEM_ID: &str = "timer.timed_choice";

/// Outcome reported when the player picks the right option.
pub const OUTCOME_CORRECT: &str = "correct";
/// Outcome reported when the player picks a wrong option.
pub const OUTCOME_WRONG: &str = "wrong";
/// Outcome reported when time runs out without a pick.
pub const OUTCOME_TIMEOUT: &str = "timeout";

/// What the script configures.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TimerConfig {
    /// Shown above the options.
    #[serde(default)]
    pub prompt: Option<String>,
    /// How long the player has. Zero means the first step times out, which is
    /// useful for testing and useless for playing.
    pub duration_ms: u32,
    /// Labels the player chooses between. Must not be empty.
    pub options: Vec<String>,
    /// Index into `options` that counts as correct.
    pub correct: usize,
}

/// Configuration errors, reported through [`CoreError::GameplayFailed`].
#[derive(Debug, thiserror::Error)]
pub enum TimerConfigError {
    #[error("config is missing; a timed choice needs duration_ms, options and correct")]
    Missing,
    #[error("failed to parse config: {0}")]
    Parse(String),
    #[error("options must not be empty")]
    NoOptions,
    #[error("correct index {correct} is out of range for {count} options")]
    CorrectOutOfRange { correct: usize, count: usize },
}

impl TimerConfig {
    /// Parse and validate, so a bad script fails on handover rather than mid-scene.
    pub fn from_value(value: Option<&ron::Value>) -> Result<Self, TimerConfigError> {
        let Some(value) = value else {
            return Err(TimerConfigError::Missing);
        };
        let config: Self = value
            .clone()
            .into_rust::<Self>()
            .map_err(|err| TimerConfigError::Parse(err.to_string()))?;

        if config.options.is_empty() {
            return Err(TimerConfigError::NoOptions);
        }
        if config.correct >= config.options.len() {
            return Err(TimerConfigError::CorrectOutOfRange {
                correct: config.correct,
                count: config.options.len(),
            });
        }
        Ok(config)
    }
}

/// The system itself.
#[derive(Debug, Default)]
pub struct TimedChoice {
    config: Option<TimerConfig>,
    remaining_ms: u32,
    chosen: Option<usize>,
}

/// What a save must remember about a timed choice.
///
/// `config` is deliberately absent: it comes back from the script through
/// [`GameplaySystem::begin`], and duplicating it here would let a save carry a
/// stale duration that disagrees with the script it is loaded against.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TimerState {
    remaining_ms: u32,
    chosen: Option<usize>,
}

impl TimedChoice {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn config(&self) -> Option<&TimerConfig> {
        self.config.as_ref()
    }

    #[must_use]
    pub fn remaining_ms(&self) -> u32 {
        self.remaining_ms
    }

    /// The option the player picked this round, if any.
    #[must_use]
    pub fn chosen(&self) -> Option<usize> {
        self.chosen
    }
}

impl GameplaySystem for TimedChoice {
    fn id(&self) -> SystemId {
        SystemId::from(SYSTEM_ID)
    }

    fn begin(&mut self, config: Option<&ron::Value>) -> Result<(), CoreError> {
        // Validation happens before any state is touched, so a rejected config
        // leaves nothing half-initialized for the next scene to inherit.
        let parsed = TimerConfig::from_value(config).map_err(|err| CoreError::GameplayFailed {
            system: SYSTEM_ID.to_owned(),
            message: err.to_string(),
        })?;

        self.remaining_ms = parsed.duration_ms;
        self.chosen = None;
        self.config = Some(parsed);
        Ok(())
    }

    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        let config = self.config.as_ref()?;

        // Input wins over the clock: if the player picked on the same frame the
        // timer hit zero, they answered in time.
        for input in ctx.inputs {
            if let GameplayInput::Select { index } = input {
                let index = *index;
                if index >= config.options.len() {
                    // Out-of-range selections are ignored rather than treated as
                    // wrong: the caller offered options this system never showed.
                    continue;
                }
                self.chosen = Some(index);
                ctx.stats.add("timed_choice.attempts", 1);
                let outcome = if index == config.correct {
                    ctx.stats.add("timed_choice.correct", 1);
                    OUTCOME_CORRECT
                } else {
                    OUTCOME_WRONG
                };
                return Some(OutcomeKey::from(outcome));
            }
        }

        self.remaining_ms = self.remaining_ms.saturating_sub(ctx.dt_ms);
        if self.remaining_ms == 0 {
            ctx.stats.add("timed_choice.attempts", 1);
            return Some(OutcomeKey::from(OUTCOME_TIMEOUT));
        }
        None
    }

    fn cancel(&mut self) {
        self.config = None;
        self.remaining_ms = 0;
        self.chosen = None;
    }

    /// Persist the countdown.
    ///
    /// A timer looks like it could be rebuilt from `Stats` alone, and a naive
    /// implementation would do exactly that. It would be wrong: `Stats` records
    /// attempts, not how much of *this* countdown is left, and reading a save would
    /// restart the clock. Anything the player can perceive as time passing belongs
    /// in here.
    fn save(&self) -> Result<Option<ron::Value>, CoreError> {
        if self.config.is_none() {
            // Never begun, nothing worth restoring.
            return Ok(None);
        }
        let state = TimerState {
            remaining_ms: self.remaining_ms,
            chosen: self.chosen,
        };
        let value = encode_state(&state).map_err(|message| CoreError::GameplaySaveFailed {
            system: SYSTEM_ID.to_owned(),
            message,
        })?;
        Ok(Some(value))
    }

    fn restore(&mut self, state: Option<&ron::Value>) -> Result<(), CoreError> {
        // `begin` has already run, so the config is present and the countdown is at
        // its full duration. Absence of state means the save predates this system
        // implementing `save`, and starting the countdown fresh is the best guess
        // available.
        let Some(value) = state else {
            return Ok(());
        };

        let state: TimerState =
            decode_state(value).map_err(|message| CoreError::GameplayRestoreFailed {
                system: SYSTEM_ID.to_owned(),
                message,
            })?;

        self.remaining_ms = state.remaining_ms;
        self.chosen = state.chosen;
        Ok(())
    }

    fn prompt(&self) -> Option<String> {
        let config = self.config.as_ref()?;
        let base = config.prompt.as_deref().unwrap_or("Choose quickly");
        // The options belong in the prompt: without them the player is asked to
        // answer something they were never shown.
        let options = config
            .options
            .iter()
            .enumerate()
            .map(|(index, option)| format!("{}. {option}", index + 1))
            .collect::<Vec<_>>()
            .join("      ");
        Some(format!(
            "{base}   ({:.1}s)\n{options}",
            self.remaining_ms as f32 / 1000.0
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_value() -> ron::Value {
        ron::from_str(
            r#"(
                prompt: Some("Pick one"),
                duration_ms: 1000,
                options: ["a", "b"],
                correct: 1,
            )"#,
        )
        .expect("test config parses")
    }

    fn ctx<'a>(
        dt_ms: u32,
        inputs: &'a [GameplayInput],
        stats: &'a mut Stats,
    ) -> GameplayContext<'a> {
        GameplayContext {
            dt_ms,
            inputs,
            stats,
        }
    }

    #[test]
    fn picking_the_right_option_reports_correct() {
        let mut system = TimedChoice::new();
        system
            .begin(Some(&config_value()))
            .expect("config is valid");
        let mut stats = Stats::default();

        let outcome = system.step(&mut ctx(
            16,
            &[GameplayInput::Select { index: 1 }],
            &mut stats,
        ));
        assert_eq!(outcome, Some(OutcomeKey::from(OUTCOME_CORRECT)));
        assert_eq!(stats.get("timed_choice.correct"), 1);
        assert_eq!(system.chosen(), Some(1));
    }

    #[test]
    fn picking_a_wrong_option_reports_wrong_without_counting_a_success() {
        let mut system = TimedChoice::new();
        system
            .begin(Some(&config_value()))
            .expect("config is valid");
        let mut stats = Stats::default();

        let outcome = system.step(&mut ctx(
            16,
            &[GameplayInput::Select { index: 0 }],
            &mut stats,
        ));
        assert_eq!(outcome, Some(OutcomeKey::from(OUTCOME_WRONG)));
        assert_eq!(stats.get("timed_choice.correct"), 0);
        assert_eq!(stats.get("timed_choice.attempts"), 1);
    }

    #[test]
    fn running_out_of_time_reports_timeout() {
        let mut system = TimedChoice::new();
        system
            .begin(Some(&config_value()))
            .expect("config is valid");
        let mut stats = Stats::default();

        assert_eq!(system.step(&mut ctx(400, &[], &mut stats)), None);
        assert_eq!(system.step(&mut ctx(400, &[], &mut stats)), None);
        assert_eq!(
            system.step(&mut ctx(400, &[], &mut stats)),
            Some(OutcomeKey::from(OUTCOME_TIMEOUT))
        );
    }

    #[test]
    fn an_answer_on_the_expiry_frame_counts_as_in_time() {
        let mut system = TimedChoice::new();
        system
            .begin(Some(&config_value()))
            .expect("config is valid");
        let mut stats = Stats::default();

        system.step(&mut ctx(999, &[], &mut stats));
        let outcome = system.step(&mut ctx(
            999,
            &[GameplayInput::Select { index: 1 }],
            &mut stats,
        ));
        assert_eq!(
            outcome,
            Some(OutcomeKey::from(OUTCOME_CORRECT)),
            "input must be read before the clock is advanced"
        );
    }

    #[test]
    fn an_option_index_this_system_never_showed_is_ignored() {
        let mut system = TimedChoice::new();
        system
            .begin(Some(&config_value()))
            .expect("config is valid");
        let mut stats = Stats::default();

        let outcome = system.step(&mut ctx(
            16,
            &[GameplayInput::Select { index: 9 }],
            &mut stats,
        ));
        assert_eq!(outcome, None, "must not resolve on an impossible index");
        assert_eq!(system.chosen(), None);
    }

    #[test]
    fn a_missing_config_fails_before_mutating_state() {
        let mut system = TimedChoice::new();
        assert!(system.begin(None).is_err());
        assert!(system.config().is_none());
    }

    #[test]
    fn a_correct_index_beyond_the_option_list_is_rejected() {
        let bad: ron::Value =
            ron::from_str(r#"(duration_ms: 100, options: ["a"], correct: 5)"#).expect("parses");
        let mut system = TimedChoice::new();
        let err = system.begin(Some(&bad)).expect_err("must reject");
        assert!(matches!(err, CoreError::GameplayFailed { .. }));
    }

    #[test]
    fn cancel_clears_everything_so_the_next_begin_is_clean() {
        let mut system = TimedChoice::new();
        system
            .begin(Some(&config_value()))
            .expect("config is valid");
        let mut stats = Stats::default();
        system.step(&mut ctx(
            16,
            &[GameplayInput::Select { index: 0 }],
            &mut stats,
        ));

        system.cancel();
        assert!(system.config().is_none());
        assert_eq!(system.chosen(), None);
        assert_eq!(system.remaining_ms(), 0);
    }

    #[test]
    fn the_prompt_reports_the_time_left_and_the_options() {
        let mut system = TimedChoice::new();
        system
            .begin(Some(&config_value()))
            .expect("config is valid");
        let prompt = system.prompt().expect("prompt");
        assert!(prompt.contains("1.0s"), "got {prompt}");
        assert!(prompt.contains("1. a"), "options must be visible: {prompt}");
        assert!(prompt.contains("2. b"), "options must be visible: {prompt}");
    }
}
