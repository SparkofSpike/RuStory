//! The script format, for the parts that are about text.
//!
//! Translations live in the same file as the story they belong to, which is a deliberate
//! choice worth restating here: a project that ships three languages has three tables and one
//! graph of nodes, and keeping them together means a node can never be edited into a shape that
//! its translations do not describe.

use rustory_vn_core::{Beat, Exit, NodeId, Text};
use rustory_vn_script::parse_story;

/// A minimal story in RON, with the parts under test spliced in.
fn script(body: &str, locales: &str) -> String {
    format!(
        r#"
Story(
    start: "only",
    nodes: {{
        "only": (
            beats: [ {body} ],
            exit: End(ending: None),
        ),
    }},{locales}
)
"#
    )
}

#[test]
fn a_line_written_out_is_unchanged_by_the_locale_feature() {
    // The compatibility promise, at the format level: every script written before keys existed
    // still parses, byte for byte.
    let story = parse_story(
        &script(r#"Say(speaker: None, text: "The bell rings.")"#, ""),
        "test.ron",
    )
    .expect("parses");

    let beats = &story.nodes[&NodeId::from("only")].beats;
    assert_eq!(
        beats[0],
        Beat::Say {
            speaker: None,
            text: Text::literal("The bell rings."),
        }
    );
    assert!(story.locales.is_empty());
}

#[test]
fn a_keyed_line_parses_and_resolves_through_its_table() {
    let story = parse_story(
        &script(
            r#"Say(speaker: None, text: (key: "only.line"))"#,
            r#"
    locales: {
        "en": { "only.line": "The bell rings." },
        "zh-Hans": { "only.line": "铃响了。" },
    },
    default_language: Some("en"),"#,
        ),
        "test.ron",
    )
    .expect("parses");

    assert_eq!(story.used_keys(), ["only.line"]);
    assert_eq!(story.languages(), ["en", "zh-Hans"]);

    let english = story.localised("en").expect("en");
    assert_eq!(
        english.nodes[&NodeId::from("only")].beats[0],
        Beat::Say {
            speaker: None,
            text: Text::literal("The bell rings."),
        }
    );

    let chinese = story.localised("zh-Hans").expect("zh");
    assert_eq!(
        chinese.nodes[&NodeId::from("only")].beats[0],
        Beat::Say {
            speaker: None,
            text: Text::literal("铃响了。"),
        }
    );
}

#[test]
fn a_key_with_no_table_is_refused_at_load() {
    // Not three scenes in, with a blank box.
    let err = parse_story(
        &script(r#"Say(speaker: None, text: (key: "only.missing"))"#, ""),
        "test.ron",
    )
    .expect_err("undefined key");

    let message = err.to_string();
    assert!(
        message.contains("only.missing"),
        "the error must name the key: {message}"
    );
}

#[test]
fn a_default_language_that_is_not_a_table_is_refused_at_load() {
    let err = parse_story(
        &script(
            r#"Say(speaker: None, text: (key: "only.line"))"#,
            r#"
    locales: { "en": { "only.line": "Hi." } },
    default_language: Some("fr"),"#,
        ),
        "test.ron",
    )
    .expect_err("fr is not there");

    assert!(err.to_string().contains("fr"), "{err}");
}

#[test]
fn choice_labels_and_prompts_take_keys_too() {
    let story = parse_story(
        r#"
Story(
    start: "pick",
    nodes: {
        "pick": (
            beats: [],
            exit: Choose(
                prompt: Some((key: "pick.prompt")),
                choices: [ (label: (key: "pick.yes"), to: "yes") ],
            ),
        ),
        "yes": ( beats: [], exit: End(ending: None) ),
    },
    locales: { "en": {
        "pick.prompt": "Well?",
        "pick.yes": "Yes",
    } },
    default_language: Some("en"),
)
"#,
        "test.ron",
    )
    .expect("parses");

    let english = story.localised("en").expect("en");
    let Exit::Choose { prompt, choices } = &english.nodes[&NodeId::from("pick")].exit else {
        panic!("expected a choice");
    };
    assert_eq!(prompt.as_ref().and_then(Text::as_literal), Some("Well?"));
    assert_eq!(choices[0].label.as_literal(), Some("Yes"));
}

#[test]
fn the_demo_story_still_parses() {
    // A canary: the format changes are only safe if the story shipped with the repository
    // keeps loading.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/demo.ron");
    let story = rustory_vn_script::load_story(&path).expect("the demo still loads");
    assert!(story.nodes.len() > 1);
    assert!(
        story.locales.is_empty(),
        "the demo is written in one language and pays nothing for the feature"
    );
}
