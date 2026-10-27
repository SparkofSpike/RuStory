//! Translations: keys, tables, and what happens when one is missing.
//!
//! The theme here is the same as the save tests: a story with a hole must fail where someone
//! can see it. A player staring at an empty dialogue box cannot tell a missing translation from
//! a broken renderer, and neither can whoever they report it to.

use std::collections::BTreeMap;

use rustory_vn_core::prelude::*;

fn say(text: Text) -> Beat {
    Beat::Say {
        speaker: None,
        text,
    }
}

fn end() -> Exit {
    Exit::End { ending: None }
}

fn nodes_of(entries: Vec<(&str, Node)>) -> BTreeMap<NodeId, Node> {
    entries
        .into_iter()
        .map(|(id, node)| (NodeId::from(id), node))
        .collect()
}

fn table(pairs: &[(&str, &str)]) -> LocaleTable {
    pairs
        .iter()
        .map(|(key, text)| ((*key).to_owned(), (*text).to_owned()))
        .collect()
}

/// One node, two lines: one literal and one keyed.
fn story_with_keys() -> Story {
    let mut story = Story::new(
        "only",
        nodes_of(vec![(
            "only",
            Node {
                beats: vec![
                    say(Text::literal("Written in the script.")),
                    say(Text::key("only.greeting")),
                ],
                exit: end(),
            },
        )]),
    );
    story
        .locales
        .insert("en".to_owned(), table(&[("only.greeting", "Hello.")]));
    story
        .locales
        .insert("zh-Hans".to_owned(), table(&[("only.greeting", "你好。")]));
    story
}

#[test]
fn a_story_without_keys_plays_as_written() {
    // The compatibility promise: a single-language project pays nothing. No table, no
    // resolution step, no `default_language` to remember.
    let story = Story::new(
        "only",
        nodes_of(vec![(
            "only",
            Node {
                beats: vec![say(Text::literal("No translations here."))],
                exit: end(),
            },
        )]),
    );

    assert!(story.used_keys().is_empty());
    assert!(story.languages().is_empty());
    assert_eq!(story.preferred_language(), None);

    let narrative = Narrative::new(story).expect("plays as written");
    assert_eq!(
        narrative.line().map(|line| line.full_text),
        Some("No translations here.")
    );
}

#[test]
fn a_keyed_story_needs_a_language_before_it_can_play() {
    let mut story = story_with_keys();
    story.default_language = None;

    let err = Narrative::new(story).expect_err("two languages and no default");
    assert!(
        matches!(&err, CoreError::AmbiguousLanguage { languages } if languages.len() == 2),
        "got {err:?}"
    );
}

#[test]
fn the_default_language_is_used_without_being_asked() {
    let mut story = story_with_keys();
    story.default_language = Some("en".to_owned());

    let narrative = Narrative::new(story).expect("resolves through the default");
    // A key reaching playback would show the player `only.greeting`.
    assert_eq!(
        narrative.line().map(|line| line.full_text),
        Some("Written in the script.")
    );
}

#[test]
fn localising_replaces_keys_and_keeps_literals() {
    let story = story_with_keys();
    let chinese = story.localised("zh-Hans").expect("has Chinese");

    let beats = &chinese.nodes[&NodeId::from("only")].beats;
    assert_eq!(beats[0], say(Text::literal("Written in the script.")));
    assert_eq!(beats[1], say(Text::literal("你好。")));

    // After localising there are no keys left, which is what lets the state machine, the
    // renderer and the save format stay ignorant of translations.
    assert!(chinese.used_keys().is_empty());
}

#[test]
fn a_missing_translation_names_the_key_and_the_language() {
    let mut story = story_with_keys();
    story.locales.get_mut("zh-Hans").expect("zh").clear();

    let err = story.localised("zh-Hans").expect_err("nothing translated");
    assert_eq!(
        err,
        CoreError::MissingTranslation {
            language: "zh-Hans".to_owned(),
            key: "only.greeting".to_owned(),
        }
    );
}

#[test]
fn asking_for_a_language_the_story_does_not_have_is_refused() {
    let story = story_with_keys();
    let err = story.localised("fr").expect_err("no French");
    assert_eq!(err, CoreError::UnknownLanguage("fr".to_owned()));
}

#[test]
fn a_key_no_language_defines_is_refused_at_load() {
    // A typo in a key is the mistake that actually happens, and it must not become a blank
    // line three scenes in.
    let mut story = Story::new(
        "only",
        nodes_of(vec![(
            "only",
            Node {
                beats: vec![say(Text::key("onlu.greeting"))],
                exit: end(),
            },
        )]),
    );
    story
        .locales
        .insert("en".to_owned(), table(&[("only.greeting", "Hi.")]));
    story.default_language = Some("en".to_owned());

    let err = Narrative::new(story).expect_err("undefined key");
    assert_eq!(
        err,
        CoreError::UndefinedKey {
            key: "onlu.greeting".to_owned()
        }
    );
}

#[test]
fn a_default_language_that_names_no_table_is_refused_at_load() {
    let mut story = story_with_keys();
    story.default_language = Some("de".to_owned());

    let err = Narrative::new(story).expect_err("de is not there");
    assert_eq!(err, CoreError::UnknownLanguage("de".to_owned()));
}

#[test]
fn choice_labels_and_prompts_are_translated_too() {
    // It would be a strange engine that translated dialogue and left the menu in English.
    let mut story = Story::new(
        "pick",
        nodes_of(vec![
            (
                "pick",
                Node {
                    beats: Vec::new(),
                    exit: Exit::Choose {
                        prompt: Some(Text::key("pick.prompt")),
                        choices: vec![Choice {
                            label: Text::key("pick.yes"),
                            to: NodeId::from("yes"),
                        }],
                    },
                },
            ),
            (
                "yes",
                Node {
                    beats: Vec::new(),
                    exit: end(),
                },
            ),
        ]),
    );
    story.locales.insert(
        "en".to_owned(),
        table(&[("pick.prompt", "Well?"), ("pick.yes", "Yes")]),
    );

    let english = story.localised("en").expect("translated");
    let Exit::Choose { prompt, choices } = &english.nodes[&NodeId::from("pick")].exit else {
        panic!("expected a choice");
    };
    assert_eq!(prompt.as_ref().and_then(Text::as_literal), Some("Well?"));
    assert_eq!(choices[0].label.as_literal(), Some("Yes"));
}

#[test]
fn used_keys_lists_each_key_once_in_reading_order() {
    let mut story = Story::new(
        "only",
        nodes_of(vec![(
            "only",
            Node {
                beats: vec![
                    say(Text::key("a")),
                    say(Text::key("b")),
                    say(Text::key("a")),
                    say(Text::literal("not a key")),
                ],
                exit: end(),
            },
        )]),
    );
    story
        .locales
        .insert("en".to_owned(), table(&[("a", "A"), ("b", "B")]));

    assert_eq!(story.used_keys(), ["a", "b"]);
}

#[test]
fn a_save_restores_the_same_place_whatever_the_language() {
    // Position is a node and a beat index — nothing about it is textual, which is why
    // switching language does not invalidate a save.
    let mut story = story_with_keys();
    story.default_language = Some("en".to_owned());

    let mut registry = GameplayRegistry::new();
    let mut english = Narrative::new(story.clone().localised("en").expect("en")).expect("plays");
    // Two clicks, the way a player moves: the first finishes revealing, the second moves on.
    english.advance().expect("finishes the first line");
    english.advance().expect("moves to the second line");
    assert_eq!(english.beat_index(), 1, "the second line is showing");
    let save = english.save(&registry).expect("saves");

    let chinese = story.localised("zh-Hans").expect("zh");
    let resumed = Narrative::from_save(chinese, &save, &mut registry).expect("resumes");

    assert_eq!(resumed.cursor(), &NodeId::from("only"));
    assert_eq!(resumed.beat_index(), 1);
    // And the line it resumes on is in the new language, not the one it was saved in.
    assert_eq!(resumed.line().map(|line| line.full_text), Some("你好。"));
}
