//! What the story asked for, as a fade state machine.
//!
//! # Why there is no device here
//!
//! This crate decides *what* should be playing and at what gain. It does not open an audio
//! device, decode a file or talk to a driver. Two reasons:
//!
//! * **Testability.** A crossfade is a timeline, and a timeline is testable only if it does
//!   not need hardware. Every behaviour in here is pinned by a test that runs in
//!   microseconds on a machine with no sound card.
//! * **The same rule as the renderer.** `rustory-vn-render` knows pixels and not plots;
//!   this crate knows gain and not stories. A host decides which file backs a track name,
//!   exactly as it decides which PNG backs a background.
//!
//! # The shape of a change
//!
//! One track at a time. Changing the music fades the current track out, then fades the new
//! one in — it does not run both at once. A parallel crossfade would need two live players
//! and two gains for a difference most listeners describe as "a smoother transition"; a
//! serial one is a single number moving in a straight line, and it is the one this engine
//! can prove correct.
//!
//! A `fade_ms` of zero skips the fades entirely, which is what a scene that cuts mid-beat
//! wants.

#![forbid(unsafe_code)]

use rustory_vn_core::NarrativeEvent;

/// The track currently audible, and how loud.
#[derive(Debug, Clone, PartialEq)]
pub struct Music {
    /// Track name as the story wrote it. The host resolves it to a file.
    pub track: String,
    /// Current gain, from 0 to 1.
    pub gain: f32,
}

/// What is playing, and what it is becoming.
///
/// Feed it [`NarrativeEvent`]s and the elapsed time; ask it what to play. It never blocks,
/// never allocates per frame, and never needs a device.
#[derive(Debug, Clone, Default)]
pub struct MusicDirector {
    state: State,
}

#[derive(Debug, Clone, Default)]
enum State {
    /// Nothing playing, nothing pending.
    #[default]
    Silent,
    /// A track is audible at a stable gain.
    Playing { track: String, gain: f32 },
    /// A track is moving toward a gain at `step_per_ms`, then becomes `then`.
    Moving {
        track: String,
        gain: f32,
        target: f32,
        step_per_ms: f32,
        then: Next,
    },
}

/// What happens when a fade finishes.
#[derive(Debug, Clone)]
enum Next {
    /// Stay on this track at this gain.
    Stay,
    /// Drop this track and start `track` (or go silent) with an optional fade-in.
    Replace { track: Option<String>, fade_ms: u32 },
}

impl MusicDirector {
    /// A director that has not been told anything yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask for `track` to be playing, fading over `fade_ms`.
    ///
    /// `None` stops the music. Asking for the track that is already playing does nothing,
    /// so a loop that re-cues the same scene music does not restart it every frame.
    pub fn cue(&mut self, track: Option<&str>, fade_ms: u32) {
        // The last thing the story said is what it wants heard. Everything below is one
        // decision: is this cue a no-op, an immediate cut, or a fade?
        if self.already_playing(track) {
            return;
        }

        // A zero fade is a cut. It is also the case where the current track has no time to
        // fade out, so the transition is one assignment rather than a state with a
        // deadline that is already past.
        if fade_ms == 0 {
            self.state = match track {
                Some(track) => State::Playing {
                    track: track.to_owned(),
                    gain: 1.0,
                },
                None => State::Silent,
            };
            return;
        }

        let Some(track) = track else {
            // Stopping: fade whatever is audible down to zero, then go silent.
            self.fade_out(
                fade_ms,
                Next::Replace {
                    track: None,
                    fade_ms,
                },
            );
            return;
        };

        match &self.state {
            State::Silent => self.begin(track, fade_ms),
            _ => self.fade_out(
                fade_ms,
                Next::Replace {
                    track: Some(track.to_owned()),
                    fade_ms,
                },
            ),
        }
    }

    /// Whether `track` is already what the director is producing.
    ///
    /// A track on its way out does not count: asking for it again mid-fade-out means the
    /// story changed its mind, and the answer is to bring it back rather than to let it die.
    fn already_playing(&self, track: Option<&str>) -> bool {
        match (&self.state, track) {
            (State::Silent, None) => true,
            (State::Silent, Some(_)) => false,
            (State::Playing { track: playing, .. }, Some(wanted)) => playing == wanted,
            (State::Playing { .. }, None) => false,
            (
                State::Moving {
                    track: playing,
                    target,
                    ..
                },
                Some(wanted),
            ) => *target > 0.0 && playing == wanted,
            (State::Moving { .. }, None) => false,
        }
    }

    /// Start fading the audible track to zero, then do `then`.
    fn fade_out(&mut self, fade_ms: u32, then: Next) {
        let (track, gain) = match &self.state {
            State::Silent => {
                self.apply_next(then);
                return;
            }
            State::Playing { track, gain } => (track.clone(), *gain),
            State::Moving { track, gain, .. } => (track.clone(), *gain),
        };
        self.state = State::Moving {
            track,
            gain,
            target: 0.0,
            // From the current gain, not from 1.0: a fade that started at half volume
            // should not take the full duration to cover half the distance.
            step_per_ms: step(gain, fade_ms),
            then,
        };
    }

    /// Begin `time`, arriving at whatever `then` says.
    fn apply_next(&mut self, then: Next) {
        self.state = match then {
            Next::Stay => State::Silent,
            Next::Replace {
                track: Some(track),
                fade_ms,
            } => {
                if fade_ms == 0 {
                    State::Playing { track, gain: 1.0 }
                } else {
                    State::Moving {
                        track,
                        gain: 0.0,
                        target: 1.0,
                        step_per_ms: step(1.0, fade_ms),
                        then: Next::Stay,
                    }
                }
            }
            Next::Replace { track: None, .. } => State::Silent,
        };
    }

    /// Advance the fade by `dt_ms` of wall clock.
    pub fn tick(&mut self, dt_ms: u32) {
        let State::Moving {
            track,
            gain,
            target,
            step_per_ms,
            then,
        } = self.state.clone()
        else {
            return;
        };

        let delta = step_per_ms * dt_ms as f32;
        let next_gain = if target > gain {
            (gain + delta).min(target)
        } else {
            (gain - delta).max(target)
        };
        let arrived = (next_gain - target).abs() < f32::EPSILON;

        if !arrived {
            self.state = State::Moving {
                track,
                gain: next_gain,
                target,
                step_per_ms,
                then,
            };
            return;
        }

        self.state = match then {
            Next::Stay => State::Playing {
                track,
                gain: target,
            },
            Next::Replace {
                track: Some(next),
                fade_ms,
            } => {
                if fade_ms == 0 {
                    State::Playing {
                        track: next,
                        gain: 1.0,
                    }
                } else {
                    State::Moving {
                        track: next,
                        gain: 0.0,
                        target: 1.0,
                        step_per_ms: step(1.0, fade_ms),
                        then: Next::Stay,
                    }
                }
            }
            Next::Replace { track: None, .. } => State::Silent,
        };
    }

    /// The track that should be audible, with the gain to play it at.
    ///
    /// Returns `None` when nothing should be heard: silence, or a fade-out that has reached
    /// zero and is waiting to be replaced.
    #[must_use]
    pub fn current(&self) -> Option<Music> {
        match &self.state {
            State::Silent => None,
            State::Playing { track, gain } => Some(Music {
                track: track.clone(),
                gain: *gain,
            }),
            State::Moving { track, gain, .. } => {
                // Reported even at gain 0: a track that is fading in is already the track the
                // story wants, and a host that was told `None` would have nothing to load.
                Some(Music {
                    track: track.clone(),
                    gain: *gain,
                })
            }
        }
    }

    /// Whether the director is doing anything at all.
    ///
    /// A host can use this to stop ticking a finished fade.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        matches!(self.state, State::Silent | State::Playing { .. })
    }

    /// Apply a narrative event. Anything that is not about music is ignored, so a host can
    /// pass every event through without filtering.
    pub fn apply(&mut self, event: &NarrativeEvent) {
        if let NarrativeEvent::MusicChanged { track, fade_ms } = event {
            self.cue(track.as_deref(), *fade_ms);
        }
    }

    fn begin(&mut self, track: &str, fade_ms: u32) {
        if fade_ms == 0 {
            self.state = State::Playing {
                track: track.to_owned(),
                gain: 1.0,
            };
        } else {
            self.state = State::Moving {
                track: track.to_owned(),
                gain: 0.0,
                target: 1.0,
                step_per_ms: step(1.0, fade_ms),
                then: Next::Stay,
            };
        }
    }
}

fn step(distance: f32, fade_ms: u32) -> f32 {
    if fade_ms == 0 {
        f32::INFINITY
    } else {
        distance / fade_ms as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_of(director: &MusicDirector) -> Option<(String, f32)> {
        director.current().map(|music| (music.track, music.gain))
    }

    #[test]
    fn a_cue_with_no_fade_is_immediate() {
        let mut director = MusicDirector::new();
        director.cue(Some("classroom"), 0);
        assert_eq!(gain_of(&director), Some(("classroom".to_owned(), 1.0)));
    }

    #[test]
    fn a_cue_with_a_fade_starts_silent_and_arrives_at_full() {
        let mut director = MusicDirector::new();
        director.cue(Some("classroom"), 1000);

        // Halfway through the fade the track is audible and half as loud, which is the
        // whole point of asking for a fade instead of a cut.
        assert_eq!(gain_of(&director), Some(("classroom".to_owned(), 0.0)));
        director.tick(500);
        let (track, gain) = gain_of(&director).expect("audible after half the fade");
        assert_eq!(track, "classroom");
        assert!((gain - 0.5).abs() < 1e-6, "gain was {gain}");

        director.tick(600);
        assert_eq!(gain_of(&director), Some(("classroom".to_owned(), 1.0)));
        assert!(
            director.is_idle(),
            "a finished fade should not need ticking"
        );
    }

    #[test]
    fn changing_the_track_fades_the_old_one_out_first() {
        let mut director = MusicDirector::new();
        director.cue(Some("classroom"), 0);
        director.cue(Some("rooftop"), 1000);

        // The outgoing track is still the one playing, and it is on its way down.
        director.tick(500);
        let (track, gain) = gain_of(&director).expect("still audible");
        assert_eq!(track, "classroom");
        assert!((gain - 0.5).abs() < 1e-6, "gain was {gain}");

        // At zero the old track is gone and the new one begins its own fade-in.
        director.tick(500);
        let after = gain_of(&director);
        assert!(
            after.is_none()
                || after
                    .as_ref()
                    .is_some_and(|(track, gain)| track == "rooftop" && *gain <= 1e-6),
            "the cutover should not leave the old track audible: {after:?}"
        );

        director.tick(1000);
        assert_eq!(gain_of(&director), Some(("rooftop".to_owned(), 1.0)));
    }

    #[test]
    fn asking_for_the_playing_track_does_not_restart_it() {
        // A story that re-cues its scene music on every visit must not fade in from zero
        // again, or the music would dip whenever the player walks through the same door.
        let mut director = MusicDirector::new();
        director.cue(Some("classroom"), 0);
        director.cue(Some("classroom"), 2000);
        assert_eq!(gain_of(&director), Some(("classroom".to_owned(), 1.0)));
        assert!(director.is_idle());
    }

    #[test]
    fn stopping_with_no_fade_is_silence_at_once() {
        let mut director = MusicDirector::new();
        director.cue(Some("classroom"), 0);
        director.cue(None, 0);
        assert_eq!(director.current(), None);
        assert!(director.is_idle());
    }

    #[test]
    fn stopping_with_a_fade_reaches_silence() {
        let mut director = MusicDirector::new();
        director.cue(Some("classroom"), 0);
        director.cue(None, 400);

        director.tick(200);
        assert!(gain_of(&director).is_some_and(|(_, gain)| (gain - 0.5).abs() < 1e-6));
        director.tick(200);
        assert_eq!(director.current(), None);
        assert!(director.is_idle());
    }

    #[test]
    fn a_cue_mid_fade_retargets_instead_of_stacking() {
        // The last thing the story said is what it wants heard. A third cue must not leave
        // a queue of tracks fading into each other.
        let mut director = MusicDirector::new();
        director.cue(Some("a"), 1000);
        director.tick(200);
        director.cue(Some("b"), 0);

        assert_eq!(gain_of(&director), Some(("b".to_owned(), 1.0)));
        assert!(director.is_idle());
    }

    #[test]
    fn ticking_never_overshoots_the_target() {
        let mut director = MusicDirector::new();
        director.cue(Some("a"), 100);
        director.tick(10_000);
        assert_eq!(gain_of(&director), Some(("a".to_owned(), 1.0)));
    }

    #[test]
    fn narrative_events_drive_the_director_and_others_are_ignored() {
        let mut director = MusicDirector::new();

        director.apply(&NarrativeEvent::BackgroundChanged {
            image: Some("bg.png".into()),
        });
        assert!(
            director.is_idle(),
            "a background change says nothing about sound"
        );

        director.apply(&NarrativeEvent::MusicChanged {
            track: Some("classroom".into()),
            fade_ms: 0,
        });
        assert_eq!(gain_of(&director), Some(("classroom".to_owned(), 1.0)));

        director.apply(&NarrativeEvent::MusicChanged {
            track: None,
            fade_ms: 0,
        });
        assert_eq!(director.current(), None);
    }
}
