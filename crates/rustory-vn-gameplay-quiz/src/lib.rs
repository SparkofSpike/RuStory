//! A multi-round quiz: the reference for gameplay state a save must carry.
//!
//! # Why this exists
//!
//! `rustory-vn-gameplay-timer` proves a system can be added without touching the
//! core, but its state is nearly trivial: a countdown and one index. A critic
//! could reasonably say the contract only accommodates simple systems.
//!
//! This crate answers that. A quiz holds a growing list of answers, a cursor into
//! a list of rounds, and a verdict that only exists at the end. None of it can be
//! squeezed into [`Stats`], which is why it also implements
//! [`GameplaySystem::save`] / [`GameplaySystem::restore`].
//!
//! # Writing one in a script
//!
//! ```ron
//! (
//!     system: "quiz.multi_round",
//!     config: Some((
//!         prompt: Some("Three questions. No going back."),
//!         rounds: [
//!             (
//!                 question: "Where were you at eight?",
//!                 options: ["In the library", "At home"],
//!                 correct: 0,
//!             ),
//!             (
//!                 question: "Who did you see?",
//!                 options: ["Nobody", "The janitor", "A teacher"],
//!                 correct: 1,
//!             ),
//!         ],
//!     )),
//!     outcomes: {
//!         "perfect": "trusted",
//!         "passed": "doubted",
//!         "failed": "accused",
//!     },
//! )
//! ```

#![forbid(unsafe_code)]

use rustory_vn_core::prelude::*;
use serde::{Deserialize, Serialize};

/// The system id scripts use.
pub const SYSTEM_ID: &str = "quiz.multi_round";

/// Every answer was right.
pub const OUTCOME_PERFECT: &str = "perfect";
/// At least half were right, but not all.
pub const OUTCOME_PASSED: &str = "passed";
/// Fewer than half were right.
pub const OUTCOME_FAILED: &str = "failed";

/// One question.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Round {
    pub question: String,
    /// Labels offered to the player. Must not be empty.
    pub options: Vec<String>,
    /// Index into `options` that counts as correct.
    pub correct: usize,
}

/// What the script configures.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct QuizConfig {
    /// Shown above every question.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Questions in order. Must not be empty.
    pub rounds: Vec<Round>,
}

/// Why a config was rejected.
#[derive(Debug, thiserror::Error)]
pub enum QuizConfigError {
    #[error("config is missing; a quiz needs a list of rounds")]
    Missing,
    #[error("failed to parse config: {0}")]
    Parse(String),
    #[error("a quiz needs at least one round")]
    NoRounds,
    #[error("round {index} offers no options")]
    NoOptions { index: usize },
    #[error("round {index} marks correct option {correct}, but offers only {count}")]
    CorrectOutOfRange {
        index: usize,
        correct: usize,
        count: usize,
    },
}

impl QuizConfig {
    /// Parse and validate, so a malformed script fails at handover rather than
    /// partway through the interrogation.
    pub fn from_value(value: Option<&ron::Value>) -> Result<Self, QuizConfigError> {
        let value = value.ok_or(QuizConfigError::Missing)?;
        let config: Self = value
            .clone()
            .into_rust()
            .map_err(|err| QuizConfigError::Parse(err.to_string()))?;

        if config.rounds.is_empty() {
            return Err(QuizConfigError::NoRounds);
        }
        for (index, round) in config.rounds.iter().enumerate() {
            if round.options.is_empty() {
                return Err(QuizConfigError::NoOptions { index });
            }
            if round.correct >= round.options.len() {
                return Err(QuizConfigError::CorrectOutOfRange {
                    index,
                    correct: round.correct,
                    count: round.options.len(),
                });
            }
        }
        Ok(config)
    }

    #[must_use]
    pub fn round_count(&self) -> usize {
        self.rounds.len()
    }
}

/// What a save has to carry.
///
/// `config` is absent on purpose: it is restored from the script by `begin`. What
/// remains is exactly the part `begin` cannot reproduce — how far the player got
/// and what they answered along the way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct QuizState {
    /// Index of the round awaiting an answer. Equals `rounds.len()` when done.
    current: usize,
    /// Answer chosen for each completed round, in order.
    answers: Vec<usize>,
}

/// The system.
#[derive(Debug, Default)]
pub struct Quiz {
    config: Option<QuizConfig>,
    current: usize,
    answers: Vec<usize>,
}

impl Quiz {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn config(&self) -> Option<&QuizConfig> {
        self.config.as_ref()
    }

    /// Which round is being asked, zero-based.
    #[must_use]
    pub fn current_round(&self) -> usize {
        self.current
    }

    /// Answers given so far, oldest first.
    #[must_use]
    pub fn answers(&self) -> &[usize] {
        &self.answers
    }

    /// How many answers were correct so far.
    #[must_use]
    pub fn correct_count(&self) -> usize {
        let Some(config) = self.config.as_ref() else {
            return 0;
        };
        self.answers
            .iter()
            .enumerate()
            .filter(|(index, answer)| {
                config
                    .rounds
                    .get(*index)
                    .is_some_and(|round| round.correct == **answer)
            })
            .count()
    }

    /// The verdict once every round is answered.
    ///
    /// `None` while rounds remain. Half or more correct passes; all correct is
    /// perfect. Written as an explicit threshold rather than a ratio comparison so
    /// integer rounding never decides a borderline result.
    fn verdict(&self) -> Option<&'static str> {
        let config = self.config.as_ref()?;
        if self.current < config.rounds.len() {
            return None;
        }
        let correct = self.correct_count();
        let total = config.rounds.len();

        if correct == total {
            Some(OUTCOME_PERFECT)
        } else if correct * 2 >= total {
            Some(OUTCOME_PASSED)
        } else {
            Some(OUTCOME_FAILED)
        }
    }
}

impl GameplaySystem for Quiz {
    fn id(&self) -> SystemId {
        SystemId::from(SYSTEM_ID)
    }

    fn begin(
        &mut self,
        config: Option<&ron::Value>,
        _narrative: &NarrativeSnapshot,
    ) -> Result<(), CoreError> {
        // Validate before touching state, so a rejected handover leaves nothing
        // half-initialised for the next scene.
        let parsed = QuizConfig::from_value(config).map_err(|err| CoreError::GameplayFailed {
            system: SYSTEM_ID.to_owned(),
            message: err.to_string(),
        })?;

        self.config = Some(parsed);
        self.current = 0;
        self.answers.clear();
        Ok(())
    }

    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        let config = self.config.as_ref()?;

        // Already finished: keep reporting nothing until the narrative moves on.
        if self.current >= config.rounds.len() {
            return None;
        }

        for input in ctx.inputs {
            let GameplayInput::Select { index } = input else {
                // Confirm has no meaning here; a quiz cannot be skipped by clicking.
                continue;
            };
            let index = *index;

            // An index this round never offered is a plumbing bug, not a player
            // mistake: the caller showed options the system did not ask for.
            if index >= config.rounds[self.current].options.len() {
                continue;
            }

            let round = &config.rounds[self.current];
            if index == round.correct {
                ctx.stats.add("quiz.correct", 1);
            }
            ctx.stats.add("quiz.answered", 1);

            self.answers.push(index);
            self.current += 1;

            if let Some(verdict) = self.verdict() {
                ctx.stats
                    .set("quiz.perfect", i32::from(verdict == OUTCOME_PERFECT));
                return Some(OutcomeKey::from(verdict));
            }
            // Still mid-quiz: no outcome yet, the narrative keeps the system.
            return None;
        }

        None
    }

    fn cancel(&mut self) {
        self.config = None;
        self.current = 0;
        self.answers.clear();
    }

    fn prompt(&self) -> Option<String> {
        let config = self.config.as_ref()?;
        let round = config.rounds.get(self.current)?;

        let header = config.prompt.as_deref().unwrap_or("Answer");
        let options = round
            .options
            .iter()
            .enumerate()
            .map(|(index, option)| format!("{}. {option}", index + 1))
            .collect::<Vec<_>>()
            .join("   ");

        Some(format!(
            "{header}  [{}/{}]\n{question}\n{options}",
            self.current + 1,
            config.rounds.len(),
            question = round.question,
        ))
    }

    fn save(&self) -> Result<Option<ron::Value>, CoreError> {
        if self.config.is_none() {
            return Ok(None);
        }
        let state = QuizState {
            current: self.current,
            answers: self.answers.clone(),
        };
        let value = encode_state(&state).map_err(|message| CoreError::GameplaySaveFailed {
            system: SYSTEM_ID.to_owned(),
            message,
        })?;
        Ok(Some(value))
    }

    fn restore(&mut self, state: Option<&ron::Value>) -> Result<(), CoreError> {
        let Some(value) = state else {
            // No saved state: `begin` already reset to the first round, which is
            // the only sensible starting point.
            return Ok(());
        };

        let state: QuizState =
            decode_state(value).map_err(|message| CoreError::GameplayRestoreFailed {
                system: SYSTEM_ID.to_owned(),
                message,
            })?;

        // A save whose round index runs past the config means the script was
        // edited. Refusing beats resuming into a round that does not exist.
        let count = self.config.as_ref().map_or(0, |config| config.rounds.len());
        if state.current > count {
            return Err(CoreError::GameplayRestoreFailed {
                system: SYSTEM_ID.to_owned(),
                message: format!(
                    "save is at round {}, but the quiz has only {count} rounds",
                    state.current
                ),
            });
        }
        if state.answers.len() != state.current {
            return Err(CoreError::GameplayRestoreFailed {
                system: SYSTEM_ID.to_owned(),
                message: format!(
                    "save holds {} answers but reports {} completed rounds",
                    state.answers.len(),
                    state.current
                ),
            });
        }

        self.current = state.current;
        self.answers = state.answers;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_ron() -> ron::Value {
        ron::from_str(
            r#"(
                prompt: Some("Three questions."),
                rounds: [
                    (question: "One?", options: ["a", "b"], correct: 0),
                    (question: "Two?", options: ["a", "b"], correct: 1),
                    (question: "Three?", options: ["a", "b"], correct: 0),
                ],
            )"#,
        )
        .expect("test config parses")
    }

    /// A snapshot with nothing in it: these tests exercise the system's own rules, and none of
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

    /// Answer the current round with `index`.
    fn answer(system: &mut Quiz, index: usize, stats: &mut Stats) -> Option<OutcomeKey> {
        system.step(&mut ctx(16, &[GameplayInput::Select { index }], stats))
    }

    #[test]
    fn a_perfect_run_reports_perfect() {
        let mut quiz = Quiz::new();
        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");
        let mut stats = Stats::default();

        assert_eq!(answer(&mut quiz, 0, &mut stats), None);
        assert_eq!(answer(&mut quiz, 1, &mut stats), None);
        let outcome = answer(&mut quiz, 0, &mut stats);

        assert_eq!(outcome, Some(OutcomeKey::from(OUTCOME_PERFECT)));
        assert_eq!(stats.get("quiz.correct"), 3);
        assert_eq!(stats.get("quiz.perfect"), 1);
    }

    #[test]
    fn a_mostly_wrong_run_reports_failed() {
        let mut quiz = Quiz::new();
        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");
        let mut stats = Stats::default();

        answer(&mut quiz, 1, &mut stats); // wrong
        answer(&mut quiz, 0, &mut stats); // wrong
        let outcome = answer(&mut quiz, 1, &mut stats); // wrong

        assert_eq!(outcome, Some(OutcomeKey::from(OUTCOME_FAILED)));
        assert_eq!(stats.get("quiz.correct"), 0);
    }

    #[test]
    fn one_right_out_of_three_fails_because_the_threshold_is_half() {
        let mut quiz = Quiz::new();
        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");
        let mut stats = Stats::default();

        answer(&mut quiz, 0, &mut stats); // right
        answer(&mut quiz, 0, &mut stats); // wrong
        let outcome = answer(&mut quiz, 1, &mut stats); // wrong

        assert_eq!(
            outcome,
            Some(OutcomeKey::from(OUTCOME_FAILED)),
            "1 of 3 is below half"
        );
    }

    #[test]
    fn the_prompt_reports_progress_question_and_options() {
        let mut quiz = Quiz::new();
        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");

        let prompt = quiz.prompt().expect("prompt");
        assert!(prompt.contains("[1/3]"), "got {prompt}");
        assert!(prompt.contains("One?"), "got {prompt}");
        assert!(prompt.contains("1. a"), "got {prompt}");
        assert!(prompt.contains("2. b"), "got {prompt}");

        answer(&mut quiz, 0, &mut Stats::default());
        let prompt = quiz.prompt().expect("prompt");
        assert!(prompt.contains("[2/3]"), "progress must advance: {prompt}");
        assert!(prompt.contains("Two?"), "got {prompt}");
    }

    #[test]
    fn confirm_does_not_advance_a_quiz() {
        let mut quiz = Quiz::new();
        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");
        let mut stats = Stats::default();

        let outcome = quiz.step(&mut ctx(16, &[GameplayInput::Confirm], &mut stats));
        assert_eq!(outcome, None);
        assert_eq!(
            quiz.current_round(),
            0,
            "a click must not answer a question"
        );
        assert_eq!(stats.get("quiz.answered"), 0);
    }

    #[test]
    fn an_option_index_this_round_never_offered_is_ignored() {
        let mut quiz = Quiz::new();
        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");
        let mut stats = Stats::default();

        let outcome = answer(&mut quiz, 9, &mut stats);
        assert_eq!(outcome, None);
        assert_eq!(quiz.current_round(), 0);
        assert!(quiz.answers().is_empty());
    }

    #[test]
    fn a_bad_config_is_rejected_before_state_changes() {
        let empty_rounds: ron::Value = ron::from_str("(rounds: [])").expect("parses");
        let mut quiz = Quiz::new();
        assert!(quiz.begin(Some(&empty_rounds), snapshot()).is_err());
        assert!(quiz.config().is_none());
        assert_eq!(quiz.current_round(), 0);

        let bad_index: ron::Value =
            ron::from_str("(rounds: [(question: \"q\", options: [\"a\"], correct: 4)])")
                .expect("parses");
        assert!(quiz.begin(Some(&bad_index), snapshot()).is_err());
        assert!(quiz.config().is_none());
    }

    // --- saving ------------------------------------------------------------

    #[test]
    fn a_half_finished_quiz_saves_and_resumes_at_the_same_round() {
        let mut quiz = Quiz::new();
        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");
        let mut stats = Stats::default();

        answer(&mut quiz, 0, &mut stats); // round 1 answered
        let saved = quiz.save().expect("saves").expect("stateful");

        let mut resumed = Quiz::new();
        resumed
            .begin(Some(&config_ron()), snapshot())
            .expect("valid config");
        assert_eq!(resumed.current_round(), 0, "begin alone starts over");

        resumed.restore(Some(&saved)).expect("restores");

        assert_eq!(resumed.current_round(), 1, "must resume at round two");
        assert_eq!(resumed.answers(), &[0], "must remember the first answer");

        // Finishing from the restored position must produce the same verdict as
        // an uninterrupted run.
        answer(&mut resumed, 1, &mut stats);
        let outcome = answer(&mut resumed, 0, &mut stats);
        assert_eq!(outcome, Some(OutcomeKey::from(OUTCOME_PERFECT)));
    }

    #[test]
    fn restoring_rejects_a_round_index_beyond_the_config() {
        let mut quiz = Quiz::new();
        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");

        let bogus = encode_state(&QuizState {
            current: 9,
            answers: vec![0, 0, 0, 0, 0, 0, 0, 0, 0],
        })
        .expect("encodes");

        let err = quiz.restore(Some(&bogus)).expect_err("must refuse");
        assert!(
            matches!(err, CoreError::GameplayRestoreFailed { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn restoring_rejects_an_answer_count_that_disagrees_with_the_round_index() {
        let mut quiz = Quiz::new();
        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");

        let bogus = encode_state(&QuizState {
            current: 2,
            answers: vec![0],
        })
        .expect("encodes");

        let err = quiz.restore(Some(&bogus)).expect_err("must refuse");
        assert!(
            matches!(err, CoreError::GameplayRestoreFailed { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_stateless_save_is_absent_before_begin() {
        let quiz = Quiz::new();
        assert!(quiz.save().expect("saves").is_none());
    }

    #[test]
    fn cancel_clears_progress_so_a_replay_starts_fresh() {
        let mut quiz = Quiz::new();
        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");
        answer(&mut quiz, 0, &mut Stats::default());

        quiz.cancel();
        assert_eq!(quiz.current_round(), 0);
        assert!(quiz.answers().is_empty());

        quiz.begin(Some(&config_ron()), snapshot())
            .expect("valid config");
        assert_eq!(quiz.current_round(), 0, "begin resets after cancel");
    }
}
