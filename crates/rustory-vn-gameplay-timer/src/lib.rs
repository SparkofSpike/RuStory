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

    fn begin(
        &mut self,
        config: Option<&ron::Value>,
        _narrative: &NarrativeSnapshot,
    ) -> Result<(), CoreError> {
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

    /// What the player sees: the question, the options, and a bar that is the clock.
    ///
    /// The bar exists because a textual "(2.4s)" is a number the player has to read and
    /// subtract from; a bar is a shape they can glance at. Both are drawn, since the number is
    /// what a screenshot test can assert on and the bar is what a person actually uses.
    fn view(&self) -> Option<GameplayView> {
        let config = self.config.as_ref()?;
        let duration = config.duration_ms.max(1) as f32;
        let fraction = self.remaining_ms as f32 / duration;
        let tone = if fraction <= 0.25 {
            Tone::Danger
        } else {
            Tone::Accent
        };

        let width = 760.0;
        let x = (VIEW_WIDTH - width) / 2.0;
        let height = 72.0;
        let gap = 16.0;
        let top = 280.0;

        let mut view = GameplayView::with_capacity(config.options.len() * 2 + 5)
            .push(Element::panel(ViewRect::full(), Tone::Panel))
            .push(Element::centered_text(
                config.prompt.as_deref().unwrap_or("Choose quickly"),
                ViewRect::new(60.0, 120.0, VIEW_WIDTH - 120.0, 48.0),
                40.0,
                Tone::Text,
            ))
            .push(Element::centered_text(
                format!("{:.1}s", self.remaining_ms as f32 / 1000.0),
                ViewRect::new(60.0, 184.0, VIEW_WIDTH - 120.0, 40.0),
                30.0,
                if fraction <= 0.25 {
                    Tone::Danger
                } else {
                    Tone::Muted
                },
            ));

        for (index, option) in config.options.iter().enumerate() {
            let rect = ViewRect::new(x, top + index as f32 * (height + gap), width, height);
            let border = if self.chosen == Some(index) {
                Tone::Accent
            } else {
                Tone::Edge
            };
            view = view
                .push(Element::outlined(rect, Tone::Raised, border))
                .push(Element::text(
                    format!("{}. {option}", index + 1),
                    ViewRect::new(
                        rect.x + 24.0,
                        rect.y + 16.0,
                        rect.width - 48.0,
                        rect.height - 32.0,
                    ),
                    30.0,
                    Tone::Text,
                ));
        }

        Some(view.push(Element::bar(
            ViewRect::new(x, 620.0, width, 14.0),
            fraction,
            tone,
        )))
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

    /// A snapshot with nothing in it: these tests are about the system's own rules, and none of
    /// them consult the story's position. One is still required, because a real system could.
    fn snapshot() -> &'static NarrativeSnapshot {
        static ONCE: std::sync::OnceLock<NarrativeSnapshot> = std::sync::OnceLock::new();
        ONCE.get_or_init(|| NarrativeSnapshot {
            node: NodeId::from("test"),
            beat_index: 0,
            beat_count: 0,
            visited: Vec::new(),
            choices: Vec::new(),
        })
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
            narrative: snapshot(),
        }
    }

    #[test]
    fn picking_the_right_option_reports_correct() {
        let mut system = TimedChoice::new();
        system
            .begin(Some(&config_value()), snapshot())
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
            .begin(Some(&config_value()), snapshot())
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
            .begin(Some(&config_value()), snapshot())
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
            .begin(Some(&config_value()), snapshot())
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
            .begin(Some(&config_value()), snapshot())
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
        assert!(system.begin(None, snapshot()).is_err());
        assert!(system.config().is_none());
    }

    #[test]
    fn a_correct_index_beyond_the_option_list_is_rejected() {
        let bad: ron::Value =
            ron::from_str(r#"(duration_ms: 100, options: ["a"], correct: 5)"#).expect("parses");
        let mut system = TimedChoice::new();
        let err = system
            .begin(Some(&bad), snapshot())
            .expect_err("must reject");
        assert!(matches!(err, CoreError::GameplayFailed { .. }));
    }

    #[test]
    fn cancel_clears_everything_so_the_next_begin_is_clean() {
        let mut system = TimedChoice::new();
        system
            .begin(Some(&config_value()), snapshot())
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
    fn the_view_shows_the_clock_as_a_bar_and_the_options_as_text() {
        let mut system = TimedChoice::new();
        system
            .begin(Some(&config_value()), snapshot())
            .expect("config is valid");

        let view = system.view().expect("a started system draws its screen");
        let texts: Vec<&str> = view.texts().collect();
        assert!(texts.iter().any(|t| t.contains("1.0s")), "got {texts:?}");
        assert!(
            texts.iter().any(|t| t.starts_with("1. a")),
            "options must be visible: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.starts_with("2. b")),
            "options must be visible: {texts:?}"
        );

        // The bar is what the old text-only prompt could not be: the clock as a shape the
        // player can glance at instead of subtracting numbers.
        let (_, full) = view.bars().next().expect("a countdown draws a bar");
        assert!((full - 1.0).abs() < 1e-6, "starts full, got {full}");

        let mut stats = Stats::default();
        system.step(&mut ctx(250, &[], &mut stats));

        let view = system.view().expect("still drawing");
        let (_, quarter_gone) = view.bars().next().expect("a bar");
        assert!(
            (quarter_gone - 0.75).abs() < 1e-6,
            "the bar must follow the clock, got {quarter_gone}"
        );
    }
}
