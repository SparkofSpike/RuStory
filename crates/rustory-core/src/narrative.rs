//! The narrative state machine.
//!
//! One cursor walks a story graph: it plays a node's beats in order, applying
//! stage effects immediately and pausing on each line so it can be revealed a
//! few characters at a time. When the beats run out the node's exit decides what
//! happens next.
//!
//! The machine never renders anything and never reads input directly. The caller
//! ticks it with elapsed milliseconds and tells it when the player advances or
//! chooses; it answers with [`NarrativeEvent`]s.
//!
//! ```
//! use rustory_core::prelude::*;
//!
//! let mut nodes = std::collections::BTreeMap::new();
//! nodes.insert(
//!     NodeId::from("a"),
//!     Node {
//!         beats: vec![Beat::Say { speaker: None, text: "hi".into() }],
//!         exit: Exit::End { ending: None },
//!     },
//! );
//! let story = Story {
//!     meta: StoryMeta::default(),
//!     start: NodeId::from("a"),
//!     nodes,
//! };
//!
//! let mut narrative = Narrative::new(story).expect("valid story");
//! assert!(matches!(narrative.mode(), Mode::Revealing));
//! ```

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::error::CoreError;
use crate::event::{NarrativeEvent, TickReport};
use crate::gameplay::{GameplayContext, GameplayInput, GameplayRegistry, Stats};
use crate::ids::{ActorId, NodeId, OutcomeKey, SystemId};
use crate::save::{GameplaySave, SAVE_VERSION, SaveState};
use crate::stage::Stage;
use crate::story::{Beat, Choice, Exit, GameplaySpec, Node, Story};

/// The most beats one call may traverse without yielding.
///
/// Nodes with no dialogue chain straight into each other, so a script that loops
/// `a -> b -> a` with empty beats would otherwise hang the caller forever. A
/// bound turns that authoring mistake into a reported error at the point it
/// happens instead of a frozen window.
const MAX_BEATS_PER_CALL: usize = 1024;

/// How many undrained events are kept before the oldest is dropped.
///
/// Nothing in this repository drains events yet - the renderer rebuilds its frame
/// from state each tick and does not need them - so an unbounded queue would grow
/// for the lifetime of the process. When a real consumer appears (audio cues,
/// achievements, a log), it will drain every frame and never reach this bound.
const MAX_PENDING_EVENTS: usize = 256;

/// What the machine is waiting for.
///
/// `Serialize`/`Deserialize` exist for saving: a resume has to know whether the
/// player was mid-line, staring at a choice, or inside a gameplay system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    /// A line is being revealed character by character.
    Revealing,
    /// The current line is fully visible; the player must advance.
    AwaitingAdvance,
    /// The node's beats are done and the player must pick a branch.
    AwaitingChoice {
        prompt: Option<String>,
        choices: Vec<Choice>,
    },
    /// A gameplay system has control. The narrative resumes on its verdict.
    Suspended { spec: GameplaySpec },
    /// The story reached an end node.
    Finished { ending: Option<String> },
}

/// The active line, as the renderer needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineView<'a> {
    pub speaker: Option<&'a ActorId>,
    /// The whole line, regardless of reveal progress.
    pub full_text: &'a str,
    /// The portion revealed so far, always on a grapheme boundary.
    pub visible: &'a str,
}

/// Drives a [`Story`].
#[derive(Debug)]
pub struct Narrative {
    story: Story,
    stage: Stage,
    stats: Stats,
    cursor: NodeId,
    beat_index: usize,
    mode: Mode,
    reveal_accum_ms: f32,
    revealed: usize,
    gameplay_begun: bool,
    events: Vec<NarrativeEvent>,
}

impl Narrative {
    /// Validate and start a story.
    ///
    /// Validation happens here, before a single frame, so a script with a broken
    /// reference fails at load with the offending node named.
    pub fn new(story: Story) -> Result<Self, CoreError> {
        story.validate()?;
        let start = story.start.clone();
        let mut narrative = Self {
            story,
            stage: Stage::default(),
            stats: Stats::default(),
            cursor: start.clone(),
            beat_index: 0,
            mode: Mode::AwaitingAdvance,
            reveal_accum_ms: 0.0,
            revealed: 0,
            gameplay_begun: false,
            events: Vec::new(),
        };
        narrative.enter_node(start)?;
        Ok(narrative)
    }

    // --- saving and loading ----------------------------------------------

    /// Capture everything needed to resume this session.
    ///
    /// The registry is consulted only when a gameplay system currently holds
    /// control, because only then is there state the narrative cannot rebuild.
    pub fn save(&self, registry: &GameplayRegistry) -> Result<SaveState, CoreError> {
        let gameplay = match &self.mode {
            Mode::Suspended { spec } => {
                let system =
                    registry
                        .get(&spec.system)
                        .ok_or_else(|| CoreError::UnknownGameplaySystem {
                            node: self.cursor.to_string(),
                            system: spec.system.to_string(),
                        })?;
                Some(GameplaySave {
                    system: spec.system.clone(),
                    state: system.save()?,
                })
            }
            _ => None,
        };

        Ok(SaveState {
            version: SAVE_VERSION,
            story_checksum: self.story.checksum(),
            cursor: self.cursor.clone(),
            beat_index: self.beat_index,
            mode: self.mode.clone(),
            revealed: self.revealed,
            stage: self.stage.clone(),
            stats: self.stats.clone(),
            gameplay,
        })
    }

    /// Rebuild a session from a save.
    ///
    /// Every check below exists because the alternative is a silent mis-restore:
    /// resuming in the wrong scene, mid-beat, with someone else's counters, and no
    /// way for the player to tell what went wrong. A refused load is recoverable;
    /// a corrupted one looks like an engine bug.
    ///
    /// No [`NarrativeEvent`]s are produced: loading is a restore, not a traversal,
    /// and a renderer replaying node-entry events would redraw a scene the player
    /// is already looking at.
    pub fn from_save(
        story: Story,
        save: &SaveState,
        registry: &mut GameplayRegistry,
    ) -> Result<Self, CoreError> {
        if save.version != SAVE_VERSION {
            return Err(CoreError::SaveVersionMismatch {
                found: save.version,
                expected: SAVE_VERSION,
            });
        }

        story.validate()?;
        let expected = story.checksum();
        if save.story_checksum != expected {
            return Err(CoreError::SaveStoryMismatch {
                found: save.story_checksum,
                expected,
            });
        }

        let node = story
            .node(&save.cursor)
            .ok_or_else(|| CoreError::SaveNodeMissing {
                node: save.cursor.to_string(),
            })?;

        if save.beat_index > node.beats.len() {
            return Err(CoreError::SaveBeatOutOfRange {
                node: save.cursor.to_string(),
                index: save.beat_index,
                count: node.beats.len(),
            });
        }

        check_position(save, node)?;

        let mut narrative = Self {
            story,
            stage: save.stage.clone(),
            stats: save.stats.clone(),
            cursor: save.cursor.clone(),
            beat_index: save.beat_index,
            mode: save.mode.clone(),
            reveal_accum_ms: 0.0,
            revealed: save.revealed,
            gameplay_begun: false,
            events: Vec::new(),
        };

        // A save may carry a reveal count from a line that was edited since. Clamp
        // rather than reject: the worst case is a line showing slightly less than
        // it did before, which the player cannot distinguish from a slow reveal.
        if let Some(line) = narrative.line() {
            let total = line.full_text.graphemes(true).count();
            narrative.revealed = narrative.revealed.min(total);
        } else {
            narrative.revealed = 0;
        }

        match (&save.mode, save.gameplay.as_ref()) {
            (Mode::Suspended { spec }, Some(saved)) => {
                let system = registry.get_mut(&spec.system).ok_or_else(|| {
                    CoreError::UnknownGameplaySystem {
                        node: save.cursor.to_string(),
                        system: spec.system.to_string(),
                    }
                })?;

                // `begin` first so config-derived invariants hold, then `restore`
                // for whatever changed during play.
                system.begin(spec.config.as_ref())?;
                system.restore(saved.state.as_ref())?;
                narrative.gameplay_begun = true;
            }
            (Mode::Suspended { spec }, None) => {
                return Err(CoreError::GameplayRestoreFailed {
                    system: spec.system.to_string(),
                    message: "save is suspended in gameplay but carries no gameplay state"
                        .to_owned(),
                });
            }
            (_, Some(saved)) => {
                // State without a suspension means the file was edited by hand or
                // written by a different build. Either way it cannot be trusted.
                return Err(CoreError::GameplayRestoreFailed {
                    system: saved.system.to_string(),
                    message: "save carries gameplay state but its mode is not suspended".to_owned(),
                });
            }
            (_, None) => {}
        }

        Ok(narrative)
    }

    #[must_use]
    pub fn story(&self) -> &Story {
        &self.story
    }

    #[must_use]
    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    #[must_use]
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    #[must_use]
    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    #[must_use]
    pub fn cursor(&self) -> &NodeId {
        &self.cursor
    }

    #[must_use]
    pub fn current_node(&self) -> &Node {
        // Safe by construction: `cursor` only ever holds a key that `enter_node`
        // looked up and `Story::validate` proved exists.
        &self.story.nodes[&self.cursor]
    }

    /// The beat the cursor sits on, if any.
    #[must_use]
    pub fn current_beat(&self) -> Option<&Beat> {
        self.current_node().beats.get(self.beat_index)
    }

    /// The active line for rendering, or `None` when no line is on screen.
    #[must_use]
    pub fn line(&self) -> Option<LineView<'_>> {
        let Beat::Say { speaker, text } = self.current_beat()? else {
            return None;
        };
        let end = text
            .grapheme_indices(true)
            .nth(self.revealed)
            .map_or(text.len(), |(offset, _)| offset);
        Some(LineView {
            speaker: speaker.as_ref(),
            full_text: text,
            visible: &text[..end],
        })
    }

    /// Drain accumulated events.
    ///
    /// Callers that do not drain are safe: the queue is bounded at
    /// [`MAX_PENDING_EVENTS`] and drops its oldest entries, because an
    /// unbounded queue would be a memory leak in any long session. Events are
    /// for live consumers reacting now, so a stale one has no value.
    pub fn take_events(&mut self) -> Vec<NarrativeEvent> {
        std::mem::take(&mut self.events)
    }

    /// How many events are waiting. Exposed so a caller can decide whether to
    /// drain without doing it.
    #[must_use]
    pub fn pending_event_count(&self) -> usize {
        self.events.len()
    }

    /// Advance the typewriter.
    ///
    /// `chars_per_sec` of `0` means "instant": the whole line appears on the
    /// first tick. That is the sane default for tests and for an accessibility
    /// setting, and it keeps the caller from special-casing it.
    pub fn tick(&mut self, dt_ms: u32, chars_per_sec: u32) -> TickReport {
        let mut report = TickReport::default();
        let Mode::Revealing = self.mode else {
            return report;
        };

        let Some(Beat::Say { text, .. }) = self.current_beat() else {
            return report;
        };
        let total = text.graphemes(true).count();

        if chars_per_sec == 0 {
            self.revealed = total;
        } else {
            self.reveal_accum_ms += dt_ms as f32;
            let target = (self.reveal_accum_ms / 1000.0 * chars_per_sec as f32).floor() as usize;
            self.revealed = target.min(total);
        }

        if self.revealed >= total {
            self.revealed = total;
            self.mode = Mode::AwaitingAdvance;
            report.line_completed = true;
            let speaker = match self.current_beat() {
                Some(Beat::Say { speaker, .. }) => speaker.clone(),
                _ => None,
            };
            self.push_event(NarrativeEvent::LineCompleted { speaker });
        }

        report.revealed = self.revealed;
        report
    }

    /// Player pressed advance.
    ///
    /// While revealing, this completes the line instead of moving on — the
    /// standard two-stage click that every visual novel uses, and the reason a
    /// fast reader is never forced to read at the author's pace.
    pub fn advance(&mut self) -> Result<(), CoreError> {
        match &self.mode {
            Mode::Revealing => {
                if let Some(Beat::Say { text, .. }) = self.current_beat() {
                    self.revealed = text.graphemes(true).count();
                }
                self.mode = Mode::AwaitingAdvance;
                let speaker = match self.current_beat() {
                    Some(Beat::Say { speaker, .. }) => speaker.clone(),
                    _ => None,
                };
                self.push_event(NarrativeEvent::LineCompleted { speaker });
            }
            Mode::AwaitingAdvance => {
                self.beat_index += 1;
                self.run_until_wait()?;
            }
            // Advancing during a choice or gameplay is a no-op: the caller has
            // nothing to act on, and silently skipping content would be worse.
            Mode::AwaitingChoice { .. } | Mode::Suspended { .. } | Mode::Finished { .. } => {}
        }
        Ok(())
    }

    /// Player picked branch `index` of the currently offered choices.
    pub fn choose(&mut self, index: usize) -> Result<(), CoreError> {
        let Mode::AwaitingChoice { choices, .. } = &self.mode else {
            return Err(CoreError::NotAwaitingChoice {
                node: self.cursor.to_string(),
            });
        };

        let Some(choice) = choices.get(index) else {
            return Err(CoreError::ChoiceOutOfRange {
                node: self.cursor.to_string(),
                index,
                count: choices.len(),
            });
        };

        let target = choice.to.clone();
        self.push_event(NarrativeEvent::ChoiceTaken {
            node: self.cursor.clone(),
            index,
        });
        self.enter_node(target)
    }

    /// Step the system that currently has control.
    ///
    /// The narrative owns the handover so a gameplay system never receives a
    /// reference to the story or the stage — see [`GameplayContext`] for exactly
    /// what it may touch.
    pub fn step_gameplay(
        &mut self,
        registry: &mut GameplayRegistry,
        dt_ms: u32,
        inputs: &[GameplayInput],
    ) -> Result<(), CoreError> {
        let Mode::Suspended { spec } = &self.mode else {
            return Ok(());
        };
        let spec = spec.clone();
        let system_id = spec.system.clone();

        let system =
            registry
                .get_mut(&system_id)
                .ok_or_else(|| CoreError::UnknownGameplaySystem {
                    node: self.cursor.to_string(),
                    system: system_id.to_string(),
                })?;

        if !self.gameplay_begun {
            system.begin(spec.config.as_ref())?;
            self.gameplay_begun = true;
        }

        // The outcome is collected before `resolve_gameplay` runs so the borrow
        // of `self.stats` ends here rather than being dropped explicitly.
        let outcome = {
            let mut ctx = GameplayContext {
                dt_ms,
                inputs,
                stats: &mut self.stats,
            };
            system.step(&mut ctx)
        };

        if let Some(outcome) = outcome {
            self.resolve_gameplay(&system_id, &spec, outcome)?;
        }
        Ok(())
    }

    /// A short prompt from the active gameplay system, if it has one.
    #[must_use]
    pub fn gameplay_prompt(&self, registry: &GameplayRegistry) -> Option<String> {
        let Mode::Suspended { spec } = &self.mode else {
            return None;
        };
        registry.prompt_of(&spec.system)
    }

    // --- internals -------------------------------------------------------

    fn resolve_gameplay(
        &mut self,
        system: &SystemId,
        spec: &GameplaySpec,
        outcome: OutcomeKey,
    ) -> Result<(), CoreError> {
        let Some(target) = spec.outcomes.get(&outcome) else {
            return Err(CoreError::UnmappedOutcome {
                node: self.cursor.to_string(),
                system: system.to_string(),
                outcome: outcome.to_string(),
            });
        };
        let target = target.clone();
        self.push_event(NarrativeEvent::GameplayFinished {
            node: self.cursor.clone(),
            system: system.clone(),
            outcome,
        });
        self.gameplay_begun = false;
        self.enter_node(target)
    }

    fn push_event(&mut self, event: NarrativeEvent) {
        // Drop the oldest rather than the newest: a consumer that wakes up late
        // cares about what just happened, not about what it already missed.
        if self.events.len() >= MAX_PENDING_EVENTS {
            self.events.remove(0);
        }
        self.events.push(event);
    }

    fn enter_node(&mut self, id: NodeId) -> Result<(), CoreError> {
        if !self.story.nodes.contains_key(&id) {
            return Err(CoreError::DanglingNodeRef {
                node: self.cursor.to_string(),
                target: id.to_string(),
            });
        }
        self.cursor = id.clone();
        self.beat_index = 0;
        self.reveal_accum_ms = 0.0;
        self.revealed = 0;
        self.gameplay_begun = false;
        self.push_event(NarrativeEvent::NodeEntered { node: id });
        self.run_until_wait()
    }

    /// Apply every beat that resolves immediately, stopping at the first one
    /// that needs the player.
    ///
    /// `goto` targets are followed in this loop rather than by recursing, so a
    /// long chain of empty nodes cannot overflow the stack.
    fn run_until_wait(&mut self) -> Result<(), CoreError> {
        for _ in 0..MAX_BEATS_PER_CALL {
            let node = &self.story.nodes[&self.cursor];
            if let Some(beat) = node.beats.get(self.beat_index) {
                match beat {
                    Beat::Say { .. } => {
                        self.mode = Mode::Revealing;
                        self.revealed = 0;
                        self.reveal_accum_ms = 0.0;
                        return Ok(());
                    }
                    Beat::ShowActor { actor, pose, slot } => {
                        let (actor, pose, slot) = (actor.clone(), pose.clone(), *slot);
                        self.stage.apply(&Beat::ShowActor {
                            actor: actor.clone(),
                            pose: pose.clone(),
                            slot,
                        });
                        self.events
                            .push(NarrativeEvent::ActorShown { actor, pose, slot });
                        self.beat_index += 1;
                    }
                    Beat::HideActor { actor } => {
                        let actor = actor.clone();
                        self.stage.apply(&Beat::HideActor {
                            actor: actor.clone(),
                        });
                        self.push_event(NarrativeEvent::ActorHidden { actor });
                        self.beat_index += 1;
                    }
                    Beat::SetBackground { image } => {
                        let image = image.clone();
                        self.stage.apply(&Beat::SetBackground {
                            image: image.clone(),
                        });
                        self.events
                            .push(NarrativeEvent::BackgroundChanged { image });
                        self.beat_index += 1;
                    }
                }
                continue;
            }

            // Beats exhausted: take the exit. Exit is cloned so the borrow of
            // `self.story` ends before we mutate state.
            let exit = node.exit.clone();
            match exit {
                Exit::End { ending } => {
                    self.mode = Mode::Finished {
                        ending: ending.clone(),
                    };
                    self.push_event(NarrativeEvent::StoryEnded { ending });
                    return Ok(());
                }
                Exit::Goto { to } => {
                    self.cursor = to;
                    self.beat_index = 0;
                    self.reveal_accum_ms = 0.0;
                    self.revealed = 0;
                    self.push_event(NarrativeEvent::NodeEntered {
                        node: self.cursor.clone(),
                    });
                }
                Exit::Choose { prompt, choices } => {
                    self.push_event(NarrativeEvent::ChoiceOffered {
                        node: self.cursor.clone(),
                        labels: choices.iter().map(|c| c.label.clone()).collect(),
                    });
                    self.mode = Mode::AwaitingChoice { prompt, choices };
                    return Ok(());
                }
                Exit::Gameplay { spec } => {
                    self.push_event(NarrativeEvent::GameplayStarted {
                        node: self.cursor.clone(),
                        system: spec.system.clone(),
                    });
                    self.mode = Mode::Suspended { spec };
                    return Ok(());
                }
            }
        }

        Err(CoreError::EndlessFlow {
            node: self.cursor.to_string(),
            limit: MAX_BEATS_PER_CALL,
        })
    }
}

/// Reject a mode that does not fit the saved position.
///
/// A save can be hand-edited, or produced by a build whose script has since been
/// replaced with one that happens to hash the same (only possible with a
/// deliberate collision). Neither should be able to put the machine into a state
/// it has no transition out of, so the pairing of mode, beat index and node exit is
/// checked explicitly.
fn check_position(save: &SaveState, node: &Node) -> Result<(), CoreError> {
    let out_of_range = || CoreError::SaveBeatOutOfRange {
        node: save.cursor.to_string(),
        index: save.beat_index,
        count: node.beats.len(),
    };

    let at_end = save.beat_index == node.beats.len();

    let fits = match &save.mode {
        // Both of these need a line under the cursor to act on.
        Mode::Revealing | Mode::AwaitingAdvance => {
            !at_end && matches!(node.beats.get(save.beat_index), Some(Beat::Say { .. }))
        }
        // These three only happen once the beats are exhausted, so the node must
        // be sitting on its exit.
        Mode::AwaitingChoice { .. } => at_end && matches!(node.exit, Exit::Choose { .. }),
        Mode::Finished { .. } => at_end && matches!(node.exit, Exit::End { .. }),
        Mode::Suspended { spec } => match &node.exit {
            Exit::Gameplay { spec: expected } if expected.system == spec.system => at_end,
            Exit::Gameplay { spec: expected } => {
                return Err(CoreError::SaveGameplayMismatch {
                    node: save.cursor.to_string(),
                    saved: spec.system.to_string(),
                    current: expected.system.to_string(),
                });
            }
            _ => {
                return Err(CoreError::SaveGameplayMismatch {
                    node: save.cursor.to_string(),
                    saved: spec.system.to_string(),
                    current: "(this node has no gameplay exit)".to_owned(),
                });
            }
        },
    };

    if fits { Ok(()) } else { Err(out_of_range()) }
}
