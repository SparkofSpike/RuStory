//! The story domain model.
//!
//! This is what a script compiles down to: a directed graph of [`Node`]s, each
//! holding an ordered list of [`Beat`]s and one [`Exit`]. The model lives in the
//! core rather than in the script crate because the state machine drives it
//! directly; the script crate's job is only to *produce* one of these from text.
//!
//! Two design points worth knowing before editing:
//!
//! * **Beats are commands, not dialogue.** `Say` renders text, `ShowActor` moves
//!   a sprite. Keeping them in one list means an author writes the scene in the
//!   order it happens, and the state machine needs only one cursor.
//! * **The core does not know any gameplay's config shape.** [`GameplaySpec`]
//!   carries an opaque [`ron::Value`] that only the registered system parses.
//!   That is what makes "add a gameplay system without touching the core" true
//!   rather than aspirational.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::{ActorId, NodeId, OutcomeKey, SystemId};
use crate::text::{Locales, Text};

/// What to do about a key the requested language does not translate.
///
/// Set by the caller, not by the story: the same file is a build artifact in one situation and a
/// work in progress in another, and only the person running it knows which.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissingText {
    /// Fail with the key named. The default, and what a shipped build wants: a hole must never
    /// look like prose.
    #[default]
    Refuse,
    /// Substitute the default language's text, then the key name itself. For a translator or
    /// writer part-way through, who needs to reach line 2 900 to check the pacing there.
    FallBack,
}

/// Where an actor stands on stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Slot {
    Left,
    Center,
    Right,
}

/// One command in a node's sequence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Beat {
    /// Speak a line. `speaker` is `None` for narration.
    Say {
        speaker: Option<ActorId>,
        text: Text,
    },
    /// Put an actor on stage, or move one already there.
    ShowActor {
        actor: ActorId,
        pose: String,
        slot: Slot,
    },
    /// Take an actor off stage. Removing an absent actor is a no-op, so scripts
    /// do not need to track stage state themselves.
    HideActor { actor: ActorId },
    /// Replace the background image. `None` clears it.
    SetBackground { image: Option<String> },
    /// Start, change or stop the music. `None` silences it.
    ///
    /// `fade_ms` is how long the change takes; zero or absent means instantly. A track name
    /// is a file stem that the host resolves, exactly like a background image: the core has
    /// no opinion about file formats or where audio lives.
    Music {
        track: Option<String>,
        #[serde(default)]
        fade_ms: Option<u32>,
    },
    /// Play a one-shot sound. Not remembered: a sound effect is an event, not a state, so
    /// loading a save in the middle of a line does not replay a door slam.
    Sound {
        clip: String,
        #[serde(default)]
        volume: Option<f32>,
    },
}

/// A branch the player can pick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub label: Text,
    pub to: NodeId,
}

/// Hand control to a gameplay system, then branch on what it reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameplaySpec {
    /// Which registered system takes over.
    pub system: SystemId,
    /// Configuration handed to that system verbatim. The core never inspects it.
    #[serde(default)]
    pub config: Option<ron::Value>,
    /// Where to go for each outcome the system may report. A system reporting a
    /// key absent from this table is an error, not a silent fallthrough —
    /// unhandled outcomes hide bugs.
    #[serde(default)]
    pub outcomes: BTreeMap<OutcomeKey, NodeId>,
}

/// What happens when a node's beats run out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Exit {
    /// The story stops here.
    End {
        #[serde(default)]
        ending: Option<String>,
    },
    /// Continue at another node.
    Goto { to: NodeId },
    /// Stop and let the player pick.
    Choose {
        #[serde(default)]
        prompt: Option<Text>,
        choices: Vec<Choice>,
    },
    /// Suspend the narrative and let a gameplay system drive.
    Gameplay { spec: GameplaySpec },
}

/// A scene: beats to play, then one exit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    #[serde(default)]
    pub beats: Vec<Beat>,
    pub exit: Exit,
}

/// Story-level metadata. Carried through for tooling; the state machine ignores it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StoryMeta {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
}

/// The whole story graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Story {
    #[serde(default)]
    pub meta: StoryMeta,
    /// Entry point. Must exist in `nodes`.
    pub start: NodeId,
    pub nodes: BTreeMap<NodeId, Node>,
    /// Translations, by language tag and then by key.
    ///
    /// Empty for a story written in one language, which is the common case and costs nothing:
    /// every `Text` is then a literal and no table is ever consulted.
    #[serde(default)]
    pub locales: Locales,
    /// Which language to play when the caller does not say. Must be a key of `locales`.
    #[serde(default)]
    pub default_language: Option<String>,
    /// Gameplay modules this story needs, by the name scripts invoke them under, and the path
    /// each one lives at.
    ///
    /// Empty for a story that only uses systems compiled into the engine, which is the common
    /// case and costs nothing. A story that names a module here is portable: the mapping travels
    /// with the file, so a cloned repository plays without a command line full of `--plugin`
    /// flags.
    ///
    /// A path is a hint, not a contract. Whoever loads the story may supply a different module
    /// for the same name — a newer build, a local override — because the name is what the script
    /// asked for and the path is only how it found one last time.
    #[serde(default)]
    pub plugins: BTreeMap<SystemId, String>,
}

impl Story {
    /// A story with no translations, which is what a single-language project writes.
    ///
    /// A constructor rather than `..Default::default()`: `start` and `nodes` are the story, and
    /// a `Default` that invented a start node would let a caller build a story that points at
    /// nothing. This one exists so adding a story-level field later does not break every test
    /// that builds one.
    #[must_use]
    pub fn new(start: impl Into<NodeId>, nodes: BTreeMap<NodeId, Node>) -> Self {
        Self {
            meta: StoryMeta::default(),
            start: start.into(),
            nodes,
            locales: Locales::new(),
            default_language: None,
            plugins: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Referential integrity, checked before the story is ever played.
    ///
    /// Every reference is verified here rather than at the point of use, so a
    /// broken script fails at load with the offending node named — not three
    /// clicks into the scene the player is watching.
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        use crate::error::CoreError;

        if !self.nodes.contains_key(&self.start) {
            return Err(CoreError::MissingStartNode(self.start.to_string()));
        }

        for (id, node) in &self.nodes {
            let dangling = |target: &NodeId| {
                if self.nodes.contains_key(target) {
                    None
                } else {
                    Some(CoreError::DanglingNodeRef {
                        node: id.to_string(),
                        target: target.to_string(),
                    })
                }
            };

            match &node.exit {
                Exit::End { .. } => {}
                Exit::Goto { to } => {
                    if let Some(err) = dangling(to) {
                        return Err(err);
                    }
                }
                Exit::Choose { choices, .. } => {
                    for choice in choices {
                        if let Some(err) = dangling(&choice.to) {
                            return Err(err);
                        }
                    }
                }
                Exit::Gameplay { spec } => {
                    for target in spec.outcomes.values() {
                        if let Some(err) = dangling(target) {
                            return Err(err);
                        }
                    }
                }
            }
        }

        // Translation integrity, checked at load for the same reason as the references: a key
        // with no text is a hole a player falls into, and finding it here means the author
        // sees it before anyone else does.
        if let Some(default) = &self.default_language
            && !self.locales.contains_key(default)
        {
            return Err(CoreError::UnknownLanguage(default.clone()));
        }
        if let Some(key) = self.unknown_keys().into_iter().next() {
            return Err(CoreError::UndefinedKey { key });
        }

        Ok(())
    }

    /// Every language the story carries, in stable order.
    #[must_use]
    pub fn languages(&self) -> Vec<&str> {
        self.locales.keys().map(String::as_str).collect()
    }

    /// Every key the script refers to, in the order a reader would meet it, without repeats.
    ///
    /// Used by the three questions worth asking about a translated story: which keys are
    /// undefined everywhere, which are missing from one language, and which language to play.
    #[must_use]
    pub fn used_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        let mut push = |text: &Text| {
            if let Some(key) = text.as_key()
                && !keys.iter().any(|seen| seen == key)
            {
                keys.push(key.to_owned());
            }
        };

        for node in self.nodes.values() {
            for beat in &node.beats {
                if let Beat::Say { text, .. } = beat {
                    push(text);
                }
            }
            match &node.exit {
                Exit::Choose { prompt, choices } => {
                    if let Some(prompt) = prompt {
                        push(prompt);
                    }
                    for choice in choices {
                        push(&choice.label);
                    }
                }
                Exit::End { .. } | Exit::Goto { .. } | Exit::Gameplay { .. } => {}
            }
        }
        keys
    }

    /// Keys that no language defines. Each one is a typo in the script.
    #[must_use]
    pub fn unknown_keys(&self) -> Vec<String> {
        crate::text::unknown_keys(&self.locales, &self.used_keys())
    }

    /// Keys `language` does not translate.
    #[must_use]
    pub fn missing_translations(&self, language: &str) -> Vec<String> {
        crate::text::missing_translations(&self.locales, language, &self.used_keys())
    }

    /// The same story with every key replaced by `language`'s text.
    ///
    /// This is the only place a key becomes a string. Everything downstream - the state
    /// machine, the renderer, the save format - works with a story that no longer has keys,
    /// which is why none of them had to learn that translations exist.
    ///
    /// Fails when `language` is not in `locales` or when one key has no translation there, and
    /// names the key: a story with a hole in it must not be playable.
    ///
    /// A translator working through a three-thousand-line story cannot play-test at line 2 900
    /// if this refuses outright, so [`Story::localised_with`] takes a policy. The default here
    /// stays the strict one, because strict is the right answer for a build.
    pub fn localised(&self, language: &str) -> Result<Self, crate::error::CoreError> {
        self.localised_with(language, MissingText::Refuse)
    }

    /// The same story, with a policy for keys `language` does not translate.
    ///
    /// [`MissingText::FallBack`] exists for the one case it is right for: a translator or writer
    /// part-way through. It substitutes the default language's text, and then the key name
    /// itself, so a missing line is visible on screen rather than blank — the whole reason the
    /// strict policy exists is that a hole must never look like prose, and a key name does not.
    ///
    /// It does not weaken the strict path. `rustory check` still reports every missing key, and a
    /// build that ships with holes is a decision someone has to make explicitly.
    pub fn localised_with(
        &self,
        language: &str,
        missing: MissingText,
    ) -> Result<Self, crate::error::CoreError> {
        let Some(table) = self.locales.get(language) else {
            return Err(crate::error::CoreError::UnknownLanguage(
                language.to_owned(),
            ));
        };

        // The fallback chain is only consulted when the requested language has no entry, and
        // only ever reads a *different* table: falling back to the same one would be a no-op
        // dressed up as a feature.
        let fallback = self
            .default_language
            .as_ref()
            .filter(|default| default.as_str() != language)
            .and_then(|default| self.locales.get(default));

        let mut localised = self.clone();
        localised.default_language = Some(language.to_owned());
        // Kept so a localised story can still answer "what languages does this exist in",
        // which a tool saving one back out needs.
        let resolve = |text: &Text| -> Result<Text, crate::error::CoreError> {
            match text.resolve(table) {
                Ok(resolved) => Ok(resolved),
                Err(key) => {
                    if missing == MissingText::Refuse {
                        return Err(crate::text::missing_translation(language, &key));
                    }
                    if let Some(resolved) = fallback.and_then(|t| t.get(&key)) {
                        return Ok(Text::Literal(resolved.clone()));
                    }
                    // The key itself, so a playtester sees which line is missing instead of an
                    // unaccounted-for blank.
                    Ok(Text::Literal(key))
                }
            }
        };

        for node in localised.nodes.values_mut() {
            for beat in &mut node.beats {
                if let Beat::Say { text, .. } = beat {
                    *text = resolve(text)?;
                }
            }
            match &mut node.exit {
                Exit::Choose { prompt, choices } => {
                    if let Some(prompt) = prompt {
                        *prompt = resolve(prompt)?;
                    }
                    for choice in choices {
                        choice.label = resolve(&choice.label)?;
                    }
                }
                Exit::End { .. } | Exit::Goto { .. } | Exit::Gameplay { .. } => {}
            }
        }

        Ok(localised)
    }

    /// The language a caller gets when it does not choose one.
    ///
    /// `default_language` when it names a table that exists, otherwise the only language there
    /// is, otherwise `None` - which means the story is not translated and should be played as
    /// written.
    #[must_use]
    pub fn preferred_language(&self) -> Option<&str> {
        if let Some(default) = &self.default_language
            && self.locales.contains_key(default)
        {
            return Some(default.as_str());
        }
        if self.locales.len() == 1 {
            return self.locales.keys().next().map(String::as_str);
        }
        None
    }

    /// Total number of beats, used by tests and progress reporting.
    #[must_use]
    pub fn beat_count(&self) -> usize {
        self.nodes.values().map(|node| node.beats.len()).sum()
    }

    /// A stable fingerprint of the story's **shape**.
    ///
    /// Saves record this so loading an old save against an edited script fails loudly instead
    /// of resuming at a node whose beats have shifted under the player.
    ///
    /// Shape, not content, and the difference is deliberate. A save remembers a node and a
    /// beat index; what matters is that the index still means the same thing, which depends
    /// on the *number and order of beats* and on where the branches go — not on the words. Two
    /// consequences follow, and both are wanted:
    ///
    /// * Rewriting dialogue, or switching language, leaves existing saves valid. Invalidating
    ///   them would punish an author for fixing a typo and a player for changing a setting.
    /// * Inserting a beat, deleting a node or repointing a branch changes the fingerprint,
    ///   because a saved index would then point somewhere else.
    ///
    /// FNV-1a over a canonical description, with `BTreeMap` iterating in key order, so the
    /// same shape always hashes the same across processes and compilations.
    ///
    /// This is a change detector, not a security primitive. Nothing here defends against a
    /// deliberately crafted collision; a save file is the player's own data.
    #[must_use]
    pub fn checksum(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };

        feed(self.start.as_str().as_bytes());
        for (id, node) in &self.nodes {
            feed(id.as_str().as_bytes());
            // A separator keeps `ab` + `c` from hashing like `a` + `bc`.
            feed(b"\x1f");
            feed(&(node.beats.len() as u64).to_le_bytes());
            for beat in &node.beats {
                feed(beat_kind(beat).as_bytes());
                feed(b"\x1f");
            }
            match &node.exit {
                Exit::End { .. } => feed(b"end"),
                Exit::Goto { to } => {
                    feed(b"goto\x1f");
                    feed(to.as_str().as_bytes());
                }
                Exit::Choose { choices, .. } => {
                    feed(b"choose\x1f");
                    feed(&(choices.len() as u64).to_le_bytes());
                    for choice in choices {
                        feed(choice.to.as_str().as_bytes());
                        feed(b"\x1f");
                    }
                }
                Exit::Gameplay { spec } => {
                    feed(b"gameplay\x1f");
                    feed(spec.system.as_str().as_bytes());
                    for (outcome, to) in &spec.outcomes {
                        feed(b"\x1f");
                        feed(outcome.as_str().as_bytes());
                        feed(b"\x1e");
                        feed(to.as_str().as_bytes());
                    }
                }
            }
            feed(b"\x1d");
        }
        hash
    }
}

/// The kind of beat, as a stable tag.
///
/// Only the variant, never its text: a rewritten line must not invalidate a save, but a beat
/// inserted before the cursor must. The tag is spelled out rather than derived from a `Debug`
/// string so a rename in the source cannot silently change what a fingerprint means.
fn beat_kind(beat: &Beat) -> &'static str {
    match beat {
        Beat::Say { .. } => "say",
        Beat::ShowActor { .. } => "show",
        Beat::HideActor { .. } => "hide",
        Beat::SetBackground { .. } => "background",
        Beat::Music { .. } => "music",
        Beat::Sound { .. } => "sound",
    }
}
