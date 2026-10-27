//! Sound output, driven by narrative events.
//!
//! # What this does and does not decide
//!
//! `rustory_audio::MusicDirector` decides *what* should be audible and at what gain; this
//! module is the part that knows about a device, files and decoders. Splitting it that way
//! keeps the fade logic testable without hardware.
//!
//! # When there is no device
//!
//! A machine with no sound card, a headless CI run, a container: all of them produce a
//! `None` device, and the game plays silently. Sound is not a reason to refuse to start —
//! but it *is* a reason to say so once, so a developer wondering why nothing is audible has
//! the answer in the log rather than in a bug report.
//!
//! # Where files come from
//!
//! `<assets>/audio/<name>.<ext>`. The name in the story is a stem, exactly like a
//! background image, and the extension is chosen by the file that is there — the engine has
//! no opinion about formats beyond what the decoder accepts.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rodio::{Decoder, MixerDeviceSink, Player, Source};
use rustory_audio::MusicDirector;
use rustory_core::NarrativeEvent;

/// Extensions the decoder is expected to handle, in the order a directory scan prefers
/// them. Order matters only when one stem exists twice; a project should not do that.
const EXTENSIONS: [&str; 4] = ["ogg", "wav", "flac", "mp3"];

/// Track names to files. Scanned once, decoded on demand: audio files are large, and a
/// story that uses a handful of them should not pay for the folder.
#[derive(Debug, Default)]
pub struct AudioBank {
    files: BTreeMap<String, PathBuf>,
}

impl AudioBank {
    /// Index a directory. A missing directory is an empty bank, not an error: a story
    /// without sound needs no `audio/` folder.
    #[must_use]
    pub fn load(dir: &Path) -> Self {
        // Every candidate is collected before one is chosen. Picking the first file the
        // directory happens to yield would make the answer depend on filesystem order,
        // which is not the same on every machine — the kind of bug that only shows up on
        // someone else's computer.
        let mut candidates: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Self::default();
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };
            if !EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            candidates.entry(stem.to_owned()).or_default().push(path);
        }

        let files = candidates
            .into_iter()
            .filter_map(|(stem, paths)| {
                let best = paths.into_iter().min_by_key(|path| rank(path))?;
                Some((stem, best))
            })
            .collect();

        Self { files }
    }

    #[must_use]
    pub fn path(&self, name: &str) -> Option<&Path> {
        self.files.get(name).map(PathBuf::as_path)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }
}

/// Where an extension sits in [`EXTENSIONS`], so a stem with several files resolves the
/// same way on every machine.
fn rank(path: &Path) -> usize {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(|ext| {
            EXTENSIONS
                .iter()
                .position(|known| known.eq_ignore_ascii_case(ext))
        })
        .unwrap_or(usize::MAX)
}

/// The audio side of a running game.
pub struct Audio {
    /// Kept alive for the whole run: dropping it stops every sound.
    device: Option<MixerDeviceSink>,
    director: MusicDirector,
    bank: AudioBank,
    /// The player carrying the music loop, if one is running.
    music: Option<Player>,
    /// Which track `music` is playing, so a director update can tell whether to swap.
    music_track: Option<String>,
    /// One-shot effects, kept until they finish so dropping them does not cut them off.
    effects: Vec<Player>,

    /// Names already reported as missing, so a repeated cue does not fill the log.
    reported: BTreeSet<String>,
    /// Why there is no sound, when there is none.
    pub unavailable: Option<String>,
}

impl Audio {
    /// Open the default device and index `dir`.
    ///
    /// Failing to open a device is recorded in [`Audio::unavailable`] rather than returned:
    /// a silent game is a degraded game, not a broken one.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        let bank = AudioBank::load(dir);
        let (device, unavailable) = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(mut device) => {
                // A normal shutdown is not news. The warning exists for a sink dropped
                // mid-playback by accident, which this is not.
                device.log_on_drop(false);
                (Some(device), None)
            }
            Err(err) => (None, Some(format!("no audio device: {err}"))),
        };

        Self {
            device,
            director: MusicDirector::new(),
            bank,
            music: None,
            music_track: None,
            effects: Vec::new(),

            reported: BTreeSet::new(),
            unavailable,
        }
    }

    /// Build one that makes no sound, for tests and for `--mute`.
    #[must_use]
    pub fn silent(dir: &Path) -> Self {
        let mut audio = Self::new(dir);
        audio.device = None;
        audio.unavailable = Some("audio disabled".to_owned());
        audio
    }

    /// How many audio files were found, for the startup report.
    #[must_use]
    pub fn library_size(&self) -> usize {
        self.bank.len()
    }

    /// Set the music to `track` immediately, without an event.
    ///
    /// Loading a save restores the track as state, not as an event — nothing "changed",
    /// the player simply arrived in a scene that was already playing. Without this, a
    /// resumed session would be silent until the next `Music` beat.
    pub fn sync_to(&mut self, track: Option<&str>) {
        // A short fade rather than a cut: the previous session's track may still be
        // playing, and a hard cut on load sounds like a glitch.
        self.director.cue(track, 400);
    }

    /// Feed events. Anything that is not about sound is ignored, so a caller can pass the
    /// whole batch without filtering.
    pub fn handle(&mut self, events: &[NarrativeEvent]) {
        for event in events {
            match event {
                NarrativeEvent::MusicChanged { .. } => self.director.apply(event),
                NarrativeEvent::SoundPlayed { clip, volume } => {
                    self.play_effect(clip, *volume);
                }
                _ => {}
            }
        }
    }

    /// Advance fades and synchronise the player with the director.
    ///
    /// Called once per frame. Cheap when nothing is fading: the director returns without
    /// allocating and the player is only touched when its gain changed.
    pub fn tick(&mut self, dt_ms: u32) {
        self.director.tick(dt_ms);

        if self.device.is_none() {
            return;
        }

        let wanted = self.director.current();
        match wanted {
            None => {
                if let Some(player) = self.music.take() {
                    player.clear();
                }
                self.music_track = None;
            }
            Some(music) => {
                let needs_swap = self.music_track.as_deref() != Some(music.track.as_str());
                if needs_swap {
                    if let Some(previous) = self.music.take() {
                        previous.clear();
                    }
                    self.music_track = None;

                    // Prepared before the device is borrowed: opening a file needs `&mut
                    // self` for the once-only warning, and the device reference is
                    // immutable, so the two cannot overlap.
                    let source = self.prepare(&music.track);
                    if let Some(source) = source
                        && let Some(device) = &self.device
                    {
                        let player = Player::connect_new(device.mixer());
                        player.append(source.repeat_infinite());
                        self.music = Some(player);
                        self.music_track = Some(music.track.clone());
                    }
                }
                if let Some(player) = &self.music {
                    player.set_volume(music.gain);
                }
            }
        }

        // Effects free themselves: a `Player` that has run out of sources is finished, and
        // holding it would keep its buffer alive for the rest of the session.
        self.effects.retain(|player| !player.empty());
    }

    /// Open and decode `name`, or explain once why it could not be done.
    fn prepare(&mut self, name: &str) -> Option<Decoder<std::io::BufReader<std::fs::File>>> {
        let path = self.resolve(name)?;
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(err) => {
                self.report_once(name, &format!("cannot open {}: {err}", path.display()));
                return None;
            }
        };
        match Decoder::try_from(std::io::BufReader::new(file)) {
            Ok(source) => Some(source),
            Err(err) => {
                self.report_once(name, &format!("cannot decode {}: {err}", path.display()));
                None
            }
        }
    }

    fn play_effect(&mut self, clip: &str, volume: f32) {
        if self.device.is_none() {
            return;
        }
        let Some(source) = self.prepare(clip) else {
            return;
        };
        let Some(device) = &self.device else {
            return;
        };

        let player = Player::connect_new(device.mixer());
        player.set_volume(volume.clamp(0.0, 1.0));
        player.append(source);
        self.effects.push(player);
    }

    /// Find a name, complaining once when it is not there.
    ///
    /// A typo in a track name is an authoring mistake, and it should be visible — but a
    /// scene that cues the same missing track every visit must not print a line each time.
    fn resolve(&mut self, name: &str) -> Option<PathBuf> {
        if let Some(path) = self.bank.path(name) {
            return Some(path.to_path_buf());
        }
        self.report_once(
            name,
            &format!(
                "no audio file for `{name}` (looked for {})",
                EXTENSIONS.join(", ")
            ),
        );
        None
    }

    fn report_once(&mut self, name: &str, message: &str) {
        if self.reported.insert(name.to_owned()) {
            eprintln!("rustory: {message}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("rustory-audio-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch dir");
            Self { path }
        }

        fn write(&self, name: &str, bytes: &[u8]) {
            std::fs::write(self.path.join(name), bytes).expect("write");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn a_missing_directory_is_an_empty_bank() {
        let bank = AudioBank::load(Path::new("definitely/not/here"));
        assert_eq!(bank.len(), 0);
    }

    #[test]
    fn files_are_indexed_by_stem_and_only_audio_extensions() {
        let scratch = Scratch::new("index");
        scratch.write("classroom.ogg", b"x");
        scratch.write("door.wav", b"x");
        scratch.write("cover.png", b"x");
        scratch.write("notes.txt", b"x");

        let bank = AudioBank::load(&scratch.path);
        assert_eq!(bank.len(), 2);
        assert!(bank.path("classroom").is_some());
        assert!(bank.path("door").is_some());
        assert!(bank.path("cover").is_none(), "a PNG is not a track");
    }

    #[test]
    fn two_files_with_one_stem_resolve_deterministically() {
        let scratch = Scratch::new("stems");
        scratch.write("theme.mp3", b"x");
        scratch.write("theme.ogg", b"x");

        let bank = AudioBank::load(&scratch.path);
        let path = bank.path("theme").expect("one of them");
        // Which one does not matter; that it does not depend on directory order does.
        assert_eq!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("ogg"),
            "the scan prefers whichever extension is listed first"
        );
    }

    #[test]
    fn a_silent_mixer_accepts_events_without_a_device() {
        let scratch = Scratch::new("silent");
        let mut audio = Audio::silent(&scratch.path);
        assert!(audio.unavailable.is_some(), "silent means it says why");

        audio.handle(&[NarrativeEvent::MusicChanged {
            track: Some("theme".into()),
            fade_ms: 100,
        }]);
        // Ticking must not panic or block when there is no device: this is the path a
        // headless CI run takes.
        audio.tick(50);
        audio.tick(50);
    }
}
