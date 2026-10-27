//! Loading stories from RON.
//!
//! This crate is deliberately thin. It exists so the core never has to know what
//! a file is, and so the format can change — or gain a second front end — without
//! disturbing the state machine.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use rustory_vn_core::{CoreError, Story};
use thiserror::Error;

/// Why a story could not be loaded.
#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path}:{line}:{column}: {message}")]
    Parse {
        path: String,
        line: usize,
        column: usize,
        message: String,
    },

    #[error("story is structurally invalid: {0}")]
    Invalid(#[from] CoreError),
}

/// Parse a story from RON text.
///
/// `origin` only appears in error messages; it is the file name the text came
/// from, so a parse failure points at a real path instead of a string blob.
pub fn parse_story(text: &str, origin: &str) -> Result<Story, ScriptError> {
    let story: Story = ron::from_str(text).map_err(|err| ScriptError::Parse {
        path: origin.to_owned(),
        line: err.span.start.line + 1,
        column: err.span.start.col + 1,
        message: err.code.to_string(),
    })?;
    story.validate()?;
    Ok(story)
}

/// Load a story from disk.
pub fn load_story(path: impl AsRef<Path>) -> Result<Story, ScriptError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| ScriptError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_story(&text, &path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustory_vn_core::{Exit, NodeId};

    const MINIMAL: &str = r#"
        Story(
            start: "a",
            nodes: {
                "a": (
                    beats: [Say(text: "hello")],
                    exit: End(),
                ),
            },
        )
    "#;

    #[test]
    fn a_minimal_story_parses() {
        let story = parse_story(MINIMAL, "minimal.ron").expect("valid");
        assert_eq!(story.start.as_str(), "a");
        assert_eq!(story.beat_count(), 1);
    }

    #[test]
    fn a_dangling_reference_fails_at_parse_time_not_at_play_time() {
        let broken = r#"
            Story(
                start: "a",
                nodes: {
                    "a": (beats: [], exit: Goto(to: "missing")),
                },
            )
        "#;
        let err = parse_story(broken, "broken.ron").expect_err("must fail");
        assert!(
            matches!(err, ScriptError::Invalid(CoreError::DanglingNodeRef { .. })),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_syntax_error_reports_a_line_and_column() {
        let err = parse_story("Story(start: ", "half.ron").expect_err("must fail");
        let ScriptError::Parse { line, column, .. } = err else {
            panic!("expected a parse error, got {err}");
        };
        assert!(line >= 1);
        assert!(column >= 1);
    }

    #[test]
    fn gameplay_config_survives_the_round_trip_untouched() {
        // The core must not interpret config, so this test only proves the value
        // arrives; the timer crate's own tests prove it is interpreted correctly.
        let text = r#"
            Story(
                start: "a",
                nodes: {
                    "a": (
                        beats: [],
                        exit: Gameplay(spec: (
                            system: "timer.timed_choice",
                            config: Some((duration_ms: 3000, options: ["x"], correct: 0)),
                            outcomes: { "correct": "a" },
                        )),
                    ),
                },
            )
        "#;
        let story = parse_story(text, "gameplay.ron").expect("valid");
        let node = story.node(&NodeId::from("a")).unwrap();
        let Exit::Gameplay { spec } = &node.exit else {
            panic!("expected a gameplay exit");
        };
        assert_eq!(spec.system.as_str(), "timer.timed_choice");
        assert!(spec.config.is_some(), "config must survive parsing");
    }
}
