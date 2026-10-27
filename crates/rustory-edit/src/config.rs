//! A gameplay config, as text.
//!
//! # Why this lives here and not in the core
//!
//! The core treats a system's config as opaque — `Option<ron::Value>` — and never inspects
//! it. That is what makes "add a gameplay without touching the core" true rather than
//! aspirational (ADR 0003). But a front-end still has to *show* the thing, and an author has
//! to be able to change it, so something must know how a config is printed and parsed.
//!
//! That something is this module, and only this module. The editor gets `to_ron` and `parse`;
//! it never learns what a `PrettyConfig` is, and the CLI's refusal and the editor's refusal
//! come from the same parser.
//!
//! # What parsing does and does not promise
//!
//! It promises the text is valid RON. It does not promise the system will accept it: only the
//! system knows its own shape, and a check here would mean the operation layer had learned
//! every system's config — the exact coupling this design exists to avoid. A wrong-but-valid
//! config fails at the handover with the system's own error, which names the field.

use rustory_core::GameplaySpec;

/// The config as RON text, pretty-printed.
///
/// Pretty rather than compact because this is what an author reads and edits: a timed choice
/// with four options on one line is unreadable, and the point of showing the config at all is
/// that it can be understood.
///
/// Returns an empty string for no config, which is what an empty text box means.
#[must_use]
pub fn to_ron(config: Option<&ron::Value>) -> String {
    let Some(value) = config else {
        return String::new();
    };
    ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default()).unwrap_or_default()
}

/// Parse RON text into a config.
///
/// Empty or whitespace-only text is `Ok(None)` rather than an error: an author who clears the
/// box means "no config", and refusing that would leave them with no way to remove one.
pub fn parse(text: &str) -> Result<Option<ron::Value>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    ron::from_str::<ron::Value>(trimmed)
        .map(Some)
        .map_err(|err| err.to_string())
}

/// The config text for a whole spec, for callers that hold one.
///
/// A convenience so a front-end does not repeat `to_ron(spec.config.as_ref())` and get the
/// `Option` nesting wrong in one of the three places it appears.
#[must_use]
pub fn spec_to_ron(spec: &GameplaySpec) -> String {
    to_ron(spec.config.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_config_is_an_empty_string_and_back_to_nothing() {
        assert_eq!(to_ron(None), "");
        assert_eq!(parse("").expect("empty is not an error"), None);
        assert_eq!(parse("   \n\t").expect("blank is not an error"), None);
    }

    #[test]
    fn a_config_survives_the_round_trip_through_text() {
        let original = parse("(duration_ms: 3000, prompt: Some(\"Choose\"))")
            .expect("parses")
            .expect("some");

        let text = to_ron(Some(&original));
        let back = parse(&text).expect("parses").expect("some");
        assert_eq!(back, original, "printing then parsing must be lossless");
    }

    #[test]
    fn text_that_is_not_ron_is_reported_rather_than_guessed_at() {
        // The mistakes that actually happen: an unbalanced bracket, a trailing comma.
        for broken in ["(duration_ms: ", "(a: 1,, b: 2)", "{"] {
            assert!(
                parse(broken).is_err(),
                "`{broken}` should not be accepted as RON"
            );
        }
    }

    #[test]
    fn a_scalar_config_is_allowed() {
        // Not every system wants a struct; `ron::Value` is happy with a number or a string,
        // and the operation layer must not invent a shape the core does not require.
        //
        // The test asserts a round trip rather than a concrete `Value`, because RON picks the
        // narrowest integer type that fits (`42` becomes `U8`) and pinning that would make a
        // harmless change in the parser look like a broken config.
        for text in ["42", "\"fast\"", "true"] {
            let parsed = parse(text)
                .unwrap_or_else(|err| panic!("`{text}` should parse: {err}"))
                .expect("some");
            let back = parse(&to_ron(Some(&parsed)))
                .expect("re-parses")
                .expect("some");
            assert_eq!(back, parsed, "`{text}` must survive printing");
        }
    }
}
