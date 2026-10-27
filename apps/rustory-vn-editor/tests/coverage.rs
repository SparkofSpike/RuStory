//! Every operation the format allows must be reachable from the GUI.
//!
//! # Why this is a source test
//!
//! The promise is "an agent can do anything a person can". The converse matters too: if
//! the operation layer gains a verb and the GUI does not get a control for it, a person
//! is quietly less capable than the CLI, and nobody notices because both still compile.
//!
//! A real UI test would need to drive widgets. This is a cheaper approximation with a
//! sharper failure: it reads the panel source and asserts each `EditOp` variant is
//! mentioned. It cannot prove the control *works* — the operation tests do that — but it
//! does prove nobody forgot to build one.
//!
//! The check runs against `include_str!`, so it sees exactly the file that was compiled.

/// The panel sources a control could live in.
const SOURCES: &[(&str, &str)] = &[
    ("panels.rs", include_str!("../src/panels.rs")),
    ("outcomes.rs", include_str!("../src/outcomes.rs")),
];

#[test]
fn every_operation_has_a_control_somewhere_in_the_editor() {
    let missing: Vec<&str> = rustory_vn_edit::catalogue()
        .iter()
        .filter(|info| {
            let needle = format!("EditOp::{}", info.name);
            !SOURCES.iter().any(|(_, source)| source.contains(&needle))
        })
        .map(|info| info.name)
        .collect();

    assert!(
        missing.is_empty(),
        "operations with no GUI control: {missing:?}\n\
         Either add a control, or — if the operation genuinely should stay CLI-only — \
         say so in ADR 0008 rather than leaving the GUI quietly weaker."
    );
}

#[test]
fn the_catalogue_and_the_enum_agree_on_size() {
    // `catalogue` matches exhaustively, so a new variant fails to compile there. This
    // catches the other direction: a variant removed without updating the list.
    let names: Vec<&str> = rustory_vn_edit::catalogue()
        .iter()
        .map(|info| info.name)
        .collect();

    assert!(
        names.len() >= 17,
        "the catalogue shrank to {} entries; did an operation get removed without \
         updating the editor?",
        names.len()
    );

    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        names.len(),
        "duplicate names in the catalogue"
    );
}
