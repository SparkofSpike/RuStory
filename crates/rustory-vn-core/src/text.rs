//! Text a player reads, and where it comes from.
//!
//! # The two forms
//!
//! A line is either written into the script or named by a key that the story's locale tables
//! resolve. Both are one type, [`Text`], because a story is usually one thing or the other and
//! never a mix of two fields that could disagree:
//!
//! ```ron
//! Say(speaker: None, text: "The bell rings."),          // literal
//! Say(speaker: None, text: (key: "prologue.bell")),     // resolved from locales
//! ```
//!
//! The literal form is untagged, so a script written before this type existed still parses.
//! That is not politeness to old files — it is the whole reason a single-language project pays
//! nothing for a feature it does not use.
//!
//! # When resolution happens
//!
//! At load, once, by [`Story::localised`]. The narrative state machine never sees a key: it
//! walks a story whose every `Text` is already a literal string. Two consequences worth stating:
//!
//! * Rendering, saving and the editor's preview need to know nothing about languages.
//! * A missing translation is a **load error with the key named**, not an empty line at
//!   runtime. A player staring at a blank box is a bug report; a load failure is a fix.
//!
//! The unlocalised story stays the editor's model — that is what an author edits, and a key
//! that resolves to nothing must be visible as a key, not silently replaced by English.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// A piece of text, written out or named by key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Text {
    /// Written into the script. What a single-language story uses, and what a translated one
    /// uses after [`Story::localised`](crate::Story::localised).
    Literal(String),
    /// A key resolved against the story's locale tables.
    Key { key: String },
}

impl Text {
    /// Text written out.
    #[must_use]
    pub fn literal(text: impl Into<String>) -> Self {
        Self::Literal(text.into())
    }

    /// Text named by a key into the locale tables.
    #[must_use]
    pub fn key(key: impl Into<String>) -> Self {
        Self::Key { key: key.into() }
    }

    /// The literal, if this is one.
    #[must_use]
    pub fn as_literal(&self) -> Option<&str> {
        match self {
            Self::Literal(text) => Some(text),
            Self::Key { .. } => None,
        }
    }

    /// The key, if this is one.
    #[must_use]
    pub fn as_key(&self) -> Option<&str> {
        match self {
            Self::Key { key } => Some(key),
            Self::Literal(_) => None,
        }
    }

    /// What to show when no locale is available.
    ///
    /// A key prints as itself. That is deliberate: an editor previewing an untranslated story
    /// should show the author the key they have to fill in, not an empty line that looks like a
    /// layout bug.
    #[must_use]
    pub fn display_without_locale(&self) -> &str {
        match self {
            Self::Literal(text) => text,
            Self::Key { key } => key,
        }
    }

    /// Replace every key with its translation from `table`.
    ///
    /// Returns the first key that is missing, so a caller can name it. Collecting all of them
    /// is [`Story::missing_translations`](crate::Story::missing_translations)'s job; this one
    /// fails fast because a half-translated story must not be played.
    pub(crate) fn resolve(&self, table: &BTreeMap<String, String>) -> Result<Self, String> {
        match self {
            Self::Literal(_) => Ok(self.clone()),
            Self::Key { key } => match table.get(key) {
                Some(text) => Ok(Self::Literal(text.clone())),
                None => Err(key.clone()),
            },
        }
    }
}

impl From<&str> for Text {
    fn from(text: &str) -> Self {
        Self::Literal(text.to_owned())
    }
}

impl From<String> for Text {
    fn from(text: String) -> Self {
        Self::Literal(text)
    }
}

impl std::fmt::Display for Text {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.display_without_locale())
    }
}

/// One language's table: key to text.
pub type LocaleTable = BTreeMap<String, String>;

/// Every language a story carries.
pub type Locales = BTreeMap<String, LocaleTable>;

/// A language tag, checked for being usable as a filename-ish identifier.
///
/// Not a BCP-47 validator: `zh-Hans`, `en`, `pt_BR` and `x-pirate` all pass, which is what
/// projects actually write. An empty tag or one with a path separator does not, because a tag
/// becomes a command-line argument and, in a tool, part of a file name.
#[must_use]
pub fn is_valid_language_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 35
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Report a key that no language defines.
#[must_use]
pub fn unknown_keys(locales: &Locales, used: &[String]) -> Vec<String> {
    used.iter()
        .filter(|key| !locales.values().any(|table| table.contains_key(*key)))
        .cloned()
        .collect()
}

/// Report a language's missing keys, in the order the story uses them.
#[must_use]
pub fn missing_translations(locales: &Locales, language: &str, used: &[String]) -> Vec<String> {
    let Some(table) = locales.get(language) else {
        return used.to_vec();
    };
    used.iter()
        .filter(|key| !table.contains_key(*key))
        .cloned()
        .collect()
}

/// The error for a key with no translation, with both names in it.
#[must_use]
pub fn missing_translation(language: &str, key: &str) -> CoreError {
    CoreError::MissingTranslation {
        language: language.to_owned(),
        key: key.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_string_parses_as_a_literal() {
        // The compatibility promise: a script written before keys existed still loads.
        let text: Text = ron::from_str("\"The bell rings.\"").expect("parses");
        assert_eq!(text, Text::literal("The bell rings."));
        assert_eq!(text.as_literal(), Some("The bell rings."));
        assert_eq!(text.as_key(), None);
    }

    #[test]
    fn a_key_parses_from_a_struct() {
        let text: Text = ron::from_str("(key: \"prologue.bell\")").expect("parses");
        assert_eq!(text, Text::key("prologue.bell"));
        assert_eq!(text.as_key(), Some("prologue.bell"));
        assert_eq!(text.as_literal(), None);
    }

    #[test]
    fn both_forms_survive_a_round_trip() {
        for original in [Text::literal("hello"), Text::key("k")] {
            let text = ron::to_string(&original).expect("serialises");
            let back: Text = ron::from_str(&text).expect("parses");
            assert_eq!(back, original, "printed as `{text}`");
        }
    }

    #[test]
    fn a_literal_prints_as_bare_text_and_a_key_prints_as_its_key() {
        // What an editor shows when no locale is chosen. A key printing as itself is the point:
        // an author should see what still needs translating.
        assert_eq!(Text::literal("hello").display_without_locale(), "hello");
        assert_eq!(
            Text::key("prologue.bell").display_without_locale(),
            "prologue.bell"
        );
    }

    #[test]
    fn resolving_replaces_a_key_and_leaves_a_literal_alone() {
        let mut table = LocaleTable::new();
        table.insert("k".to_owned(), "translated".to_owned());

        assert_eq!(
            Text::key("k").resolve(&table),
            Ok(Text::literal("translated"))
        );
        assert_eq!(
            Text::key("missing").resolve(&table),
            Err("missing".to_owned())
        );
        assert_eq!(
            Text::literal("as written").resolve(&table),
            Ok(Text::literal("as written"))
        );
    }

    #[test]
    fn language_tags_are_checked_for_being_usable() {
        for good in ["en", "zh-Hans", "pt_BR", "x-pirate"] {
            assert!(is_valid_language_tag(good), "`{good}` should be accepted");
        }
        for bad in ["", "a/b", "en zh", &"x".repeat(40)] {
            assert!(!is_valid_language_tag(bad), "`{bad}` should be refused");
        }
    }

    #[test]
    fn missing_keys_are_reported_per_language_in_use_order() {
        let mut locales = Locales::new();
        locales.insert(
            "en".to_owned(),
            LocaleTable::from([("a".to_owned(), "A".to_owned())]),
        );
        locales.insert(
            "zh".to_owned(),
            LocaleTable::from([("a".to_owned(), "A".to_owned())]),
        );

        let used = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(missing_translations(&locales, "en", &used), ["b"]);
        assert_eq!(
            missing_translations(&locales, "fr", &used),
            ["a", "b"],
            "a language that does not exist is missing everything"
        );
        assert_eq!(
            unknown_keys(&locales, &used),
            ["b"],
            "a key no language defines is a typo in the script"
        );
    }
}
