//! The WebAssembly tier: a guest module behaves like any other gameplay system, and the host
//! stays alive whatever the guest does.
//!
//! The guests here are hand-written WAT. They are deliberately crude — a module that counts to
//! three proves the ABI, and a module that loops forever proves the sandbox — because a real
//! guest SDK is a separate artefact and testing it would test the SDK.

use rustory_vn_core::prelude::*;
use rustory_vn_gameplay_wasm::{WasmError, WasmGameplay};

/// Counts three steps and then reports `done`.
const COUNTER: &str = r#"
(module
  (memory (export "memory") 1)
  (global $count (mut i32) (i32.const 0))
  (data (i32.const 1024) "done")
  (func (export "alloc") (param i32) (result i32) (i32.const 2048))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "begin") (param i32) (param i32) (result i32)
    (global.set $count (i32.const 0))
    (i32.const 0))
  (func (export "step") (param i32) (param i32) (param i32) (result i32)
    (global.set $count (i32.add (global.get $count) (i32.const 1)))
    (if (result i32) (i32.ge_s (global.get $count) (i32.const 3))
      (then (i32.const 1))
      (else (i32.const 0))))
  (func (export "result_ptr") (result i32) (i32.const 1024))
  (func (export "result_len") (result i32) (i32.const 4))
  (func (export "cancel") (global.set $count (i32.const 0)))
)
"#;

/// The same shape, but it also draws an empty view.
const WITH_VIEW: &str = r#"
(module
  (memory (export "memory") 1)
  (data (i32.const 1024) "(elements:[])")
  (func (export "alloc") (param i32) (result i32) (i32.const 2048))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "begin") (param i32) (param i32) (result i32) (i32.const 0))
  (func (export "step") (param i32) (param i32) (param i32) (result i32) (i32.const 0))
  (func (export "result_ptr") (result i32) (i32.const 1024))
  (func (export "result_len") (result i32) (i32.const 13))
  (func (export "cancel"))
  (func (export "view") (result i32) (i32.const 1))
)
"#;

/// Reports a save state and accepts one.
const WITH_STATE: &str = r#"
(module
  (memory (export "memory") 1)
  (data (i32.const 1024) "(n:1)")
  (func (export "alloc") (param i32) (result i32) (i32.const 2048))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "begin") (param i32) (param i32) (result i32) (i32.const 0))
  (func (export "step") (param i32) (param i32) (param i32) (result i32) (i32.const 0))
  (func (export "result_ptr") (result i32) (i32.const 1024))
  (func (export "result_len") (result i32) (i32.const 5))
  (func (export "cancel"))
  (func (export "save") (result i32) (i32.const 1))
  (func (export "restore") (param i32) (param i32) (result i32) (i32.const 0))
)
"#;

/// Loops forever in `begin`. Only the fuel budget can stop this.
const SPINNER: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "alloc") (param i32) (result i32) (i32.const 2048))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "begin") (param i32) (param i32) (result i32)
    (loop $forever (br $forever))
    (i32.const 0))
  (func (export "step") (param i32) (param i32) (param i32) (result i32) (i32.const 0))
  (func (export "result_ptr") (result i32) (i32.const 1024))
  (func (export "result_len") (result i32) (i32.const 0))
  (func (export "cancel"))
)
"#;

/// Missing `step`.
const NO_STEP: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "alloc") (param i32) (result i32) (i32.const 2048))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "begin") (param i32) (param i32) (result i32) (i32.const 0))
  (func (export "result_ptr") (result i32) (i32.const 1024))
  (func (export "result_len") (result i32) (i32.const 0))
  (func (export "cancel"))
)
"#;

fn snapshot() -> NarrativeSnapshot {
    NarrativeSnapshot {
        node: NodeId::from("test"),
        beat_index: 0,
        beat_count: 0,
        visited: vec![NodeId::from("test")],
        choices: Vec::new(),
    }
}

fn start(wasm: &str) -> WasmGameplay {
    let mut system = WasmGameplay::new(wasm.as_bytes()).expect("module loads");
    system
        .begin(None, &snapshot())
        .expect("begin accepts no config");
    system
}

#[test]
fn a_counter_guest_reports_an_outcome_after_three_steps() {
    let mut system = start(COUNTER);
    let position = snapshot();
    let mut stats = Stats::default();

    let mut step = |system: &mut WasmGameplay| {
        system.step(&mut GameplayContext {
            dt_ms: 16,
            inputs: &[],
            stats: &mut stats,
            narrative: &position,
        })
    };

    assert_eq!(step(&mut system), None);
    assert_eq!(step(&mut system), None);
    assert_eq!(
        step(&mut system),
        Some(OutcomeKey::from("done")),
        "the third step is the one the guest reports"
    );
    assert_eq!(
        step(&mut system),
        Some(OutcomeKey::from("done")),
        "a finished system keeps reporting its outcome, as the trait says"
    );
}

#[test]
fn a_guest_without_view_draws_nothing() {
    let system = start(COUNTER);
    assert!(
        system.view().is_none(),
        "no `view` export means no screen, not an error"
    );
}

#[test]
fn a_guest_can_return_a_view() {
    let system = start(WITH_VIEW);
    let view = system.view().expect("the guest exports `view`");
    assert!(view.is_empty(), "the guest drew nothing, which is legal");
}

#[test]
fn a_guest_save_state_round_trips_as_ron() {
    let mut system = start(WITH_STATE);
    let state = system.save().expect("save succeeds").expect("some state");
    assert!(
        matches!(&state, ron::Value::Map(map) if map.len() == 1),
        "the guest returned `(n:1)`, got {state:?}"
    );
    system
        .restore(Some(&state))
        .expect("a guest that exports restore accepts its own state");
}

#[test]
fn restore_is_refused_when_the_guest_cannot_restore() {
    let mut system = start(COUNTER);
    let state = ron::from_str("(n:1)").expect("valid RON");
    let error = system.restore(Some(&state)).expect_err("no restore export");
    assert!(
        error.to_string().contains("restore"),
        "the error must name what is missing, got {error}"
    );
}

#[test]
fn a_guest_that_loops_forever_is_stopped_by_fuel() {
    let mut system = WasmGameplay::new(SPINNER.as_bytes()).expect("module loads");
    eprintln!("loaded spinner");
    let error = system
        .begin(None, &snapshot())
        .expect_err("an infinite loop cannot complete");
    assert!(
        error.to_string().contains("out of fuel"),
        "the host must stop the guest, not hang: {error}"
    );
}

#[test]
fn a_module_missing_a_required_export_fails_to_load() {
    let error = WasmGameplay::new(NO_STEP.as_bytes()).expect_err("no `step`");
    assert!(matches!(error, WasmError::MissingExport(name) if name == "step"));
}

#[test]
fn cancel_returns_the_system_to_unstarted() {
    let mut system = start(COUNTER);
    system.cancel();
    let position = snapshot();
    let mut stats = Stats::default();
    assert_eq!(
        system.step(&mut GameplayContext {
            dt_ms: 16,
            inputs: &[],
            stats: &mut stats,
            narrative: &position,
        }),
        None,
        "a cancelled system does nothing until it begins again"
    );
}
