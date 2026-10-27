//! A key with no translation: refused by default, substituted on request.
//!
//! The theme: strict is the right answer for a build, and the wrong answer for someone at line
//! 2 900 of a first draft. The policy is the caller's, and it is a two-variant enum so a caller
//! cannot invent a third behaviour by accident.

use rustory_vn_core::prelude::*;
use std::collections::BTreeMap;

fn say(text: Text) -> Beat {
    Beat::Say {
        speaker: None,
        text,
    }
}

/// A story in two languages, with one key present in both and one only in `en`.
fn partial() -> Story {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        NodeId::from("start"),
        Node {
            beats: vec![say(Text::key("greeting")), say(Text::key("untranslated"))],
            exit: Exit::End { ending: None },
        },
    );

    let mut locales = Locales::new();
    locales.insert(
        "en".to_owned(),
        LocaleTable::from([
            ("greeting".to_owned(), "Hello".to_owned()),
            ("untranslated".to_owned(), "Only in English".to_owned()),
        ]),
    );
    locales.insert(
        "ja".to_owned(),
        LocaleTable::from([("greeting".to_owned(), "こんにちは".to_owned())]),
    );

    let mut story = Story::new("start", nodes);
    story.locales = locales;
    story.default_language = Some("en".to_owned());
    story
}

fn lines(story: &Story) -> Vec<String> {
    story.nodes[&NodeId::from("start")]
        .beats
        .iter()
        .filter_map(|beat| match beat {
            Beat::Say { text, .. } => Some(text.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_default_policy_refuses_and_names_the_key() {
    let story = partial();
    let error = story
        .localised("ja")
        .expect_err("`untranslated` has no ja text");

    let message = error.to_string();
    assert!(message.contains("untranslated"), "got {message}");
    assert!(message.contains("ja"), "got {message}");
}

#[test]
fn falling_back_substitutes_the_default_language() {
    let story = partial();
    let localised = story
        .localised_with("ja", MissingText::FallBack)
        .expect("falling back cannot fail on a missing key");

    assert_eq!(
        lines(&localised),
        ["こんにちは", "Only in English"],
        "the translated key takes the translation, the missing one takes the default"
    );
}

#[test]
fn falling_back_to_the_requested_language_is_not_treated_as_a_fallback() {
    // A story whose default language is the one being requested must behave exactly like the
    // strict path: there is nothing to fall back *to*.
    let story = partial();
    let localised = story
        .localised_with("en", MissingText::FallBack)
        .expect("en is complete");
    assert_eq!(lines(&localised), ["Hello", "Only in English"]);

    // And a story with no default language at all still has to name the key in the text.
    let mut orphan = partial();
    orphan.default_language = None;
    let localised = orphan
        .localised_with("ja", MissingText::FallBack)
        .expect("no default means no fallback, not a failure");
    assert_eq!(
        lines(&localised),
        ["こんにちは", "untranslated"],
        "with nothing to fall back to, the key itself is shown — never a blank line"
    );
}

#[test]
fn a_language_that_does_not_exist_is_still_an_error_under_either_policy() {
    // Falling back is about a missing *line*, not a missing language: silently playing English
    // when the player asked for French is a different, worse mistake.
    let story = partial();
    for policy in [MissingText::Refuse, MissingText::FallBack] {
        let error = story
            .localised_with("fr", policy)
            .expect_err("no fr table at all");
        assert!(
            matches!(&error, CoreError::UnknownLanguage(tag) if tag == "fr"),
            "got {error}"
        );
    }
}

#[test]
fn the_strict_path_is_unchanged_for_a_complete_story() {
    let story = partial();
    let strict = story.localised("en").expect("en is complete");
    let lenient = story
        .localised_with("en", MissingText::FallBack)
        .expect("en is complete");

    assert_eq!(lines(&strict), lines(&lenient));
    assert_eq!(strict.default_language, lenient.default_language);
}

#[test]
fn a_fallback_never_leaves_an_empty_line() {
    // The reason the strict policy exists is that a hole must not look like prose. A key name
    // does not look like prose, so it is the right last resort.
    let mut story = partial();
    story.locales.remove("en");
    story.default_language = Some("ja".to_owned());

    let localised = story
        .localised_with("ja", MissingText::FallBack)
        .expect("ja resolves what it has");
    for line in lines(&localised) {
        assert!(!line.is_empty(), "no line may be blank");
    }
}
