//! A gameplay system that runs a WebAssembly guest.
//!
//! This is the third tier from ADR 0005: a module loaded at run time, written by someone the
//! engine does not trust, running on top of the same `GameplaySystem` contract as the compiled-in
//! systems. It is not a different kind of gameplay; it is a different way of shipping one.
//!
//! # The boundary
//!
//! The guest gets, and can do, exactly what `GameplayContext` already gives a Rust system:
//!
//! | Direction | What crosses | Encoding |
//! |---|---|---|
//! | host → guest | config plus `NarrativeSnapshot`, once, at `begin` | RON |
//! | host → guest | `dt_ms` and the frame's inputs, every step | RON |
//! | guest → host | an outcome key, when it finishes | UTF-8 |
//! | guest → host | an optional `GameplayView` and an optional save state | RON |
//!
//! Nothing else. No WASI, no imports at all: a module that asks for `env.open_file` fails to
//! instantiate, which is a stronger guarantee than a permission check because there is nothing
//! to check. Time comes from `dt_ms`, so a guest cannot notice how long it ran.
//!
//! # What keeps the host alive
//!
//! Fuel. Every call gets a fresh budget; a guest that loops forever is stopped by the runtime
//! with [`WasmError::OutOfFuel`] rather than hanging the window. Payload sizes are capped at both
//! ends, so a module cannot ask the host to allocate a gigabyte or hand back a result the host
//! would read as a huge buffer.
//!
//! # Writing a guest
//!
//! The ABI is deliberately small enough to implement by hand. In Rust, a guest crate with
//! `crate-type = ["cdylib"]` targeting `wasm32-unknown-unknown` exports:
//!
//! ```text
//! memory                       the linear memory
//! alloc(len: i32) -> i32       a buffer the host may write into, -1 on failure
//! dealloc(ptr: i32, len: i32)  release it
//! begin(ptr: i32, len: i32) -> i32   0 = ok; payload is RON
//! step(dt: i32, ptr: i32, len: i32) -> i32   0 = keep going, 1 = finished
//! result_ptr() -> i32          where the outcome (or view, or saved state) starts
//! result_len() -> i32          how long it is
//! cancel()
//! view() -> i32                optional: 0 = nothing to draw, 1 = RON in the result buffer
//! save() -> i32                optional: 0 = stateless, 1 = RON in the result buffer
//! restore(ptr: i32, len: i32) -> i32  optional: 0 = ok
//! ```
//!
//! `view`, `save` and `restore` are optional; omitting them means "nothing to draw", "no state"
//! and "cannot restore", which is the same default a Rust system gets from the trait.

#![forbid(unsafe_code)]

use std::cell::RefCell;

use rustory_vn_core::prelude::*;
use wasmi::{Engine, Instance, Linker, Module, Store, TrapCode, Val};

/// The system id scripts use for a module loaded under the default name.
pub const SYSTEM_ID: &str = "wasm.plugin";

/// Instruction budget for one host call. Generous for real work, small enough that a runaway
/// loop is stopped in milliseconds rather than freezing the window.
pub const DEFAULT_FUEL: u64 = 4_096;

/// Largest config payload the host will hand a guest. A story config is kilobytes.
pub const MAX_CONFIG_BYTES: usize = 1 << 20;

/// Largest result the host will read back. A view with a hundred elements is far below this.
pub const MAX_RESULT_BYTES: usize = 64 << 10;

/// Why a module could not be loaded, or a call into it failed.
#[derive(Debug, thiserror::Error)]
pub enum WasmError {
    #[error("cannot load the module: {0}")]
    Load(String),
    #[error("the module does not export `{0}`")]
    MissingExport(String),
    #[error("the guest failed: {0}")]
    Guest(String),
    #[error("the guest ran out of fuel; the host stopped it")]
    OutOfFuel,
    #[error("the payload is {0} bytes, over the {1}-byte limit")]
    TooLarge(usize, usize),
    #[error("the guest's {what} is not valid RON: {message}")]
    BadPayload { what: &'static str, message: String },
}

/// Host-side state stored in the wasm store.
///
/// Empty today. It exists because the store requires a type, and because a future host
/// function — a log line, say — would need somewhere to write.
#[derive(Default)]
struct HostState;

/// A gameplay system backed by a WebAssembly module.
pub struct WasmGameplay {
    engine: Engine,
    module: Module,
    // `view(&self)` has to execute guest code, and a wasm call needs a mutable store. The
    // alternative — caching a view computed during `step` — would couple the engine's call
    // order to the view being correct, and the editor's preview calls `view` without stepping.
    store: RefCell<Store<HostState>>,
    instance: Instance,
    /// Whether begin has run. Separate from config, because a system may begin with no
    /// config at all, and Option<Option<T>> would make every call site ask which None it is.
    started: bool,
    config: Option<ron::Value>,
    finished: Option<OutcomeKey>,
}

impl std::fmt::Debug for WasmGameplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmGameplay")
            .field("started", &self.started)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl WasmGameplay {
    /// Compile a module and check the exports the protocol requires.
    ///
    /// Failing here rather than at the first handover matters: a broken module should be a load
    /// error a creator can act on, not a mid-scene failure.
    pub fn new(wasm: &[u8]) -> Result<Self, WasmError> {
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm).map_err(|err| WasmError::Load(err.to_string()))?;

        let mut store = Store::new(&engine, HostState);
        store
            .set_fuel(DEFAULT_FUEL)
            .map_err(|err| WasmError::Load(err.to_string()))?;
        let linker: Linker<HostState> = Linker::new(&engine);
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|err| WasmError::Load(err.to_string()))?;

        if instance.get_memory(&store, "memory").is_none() {
            return Err(WasmError::MissingExport("memory".to_owned()));
        }
        for name in [
            "alloc",
            "dealloc",
            "begin",
            "step",
            "result_ptr",
            "result_len",
            "cancel",
        ] {
            if instance.get_func(&store, name).is_none() {
                return Err(WasmError::MissingExport(name.to_owned()));
            }
        }

        Ok(Self {
            engine,
            module,
            store: RefCell::new(store),
            instance,
            started: false,
            config: None,
            finished: None,
        })
    }

    /// Load a module from a file.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, WasmError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .map_err(|err| WasmError::Load(format!("cannot read {}: {err}", path.display())))?;
        Self::new(&bytes)
    }

    /// The engine, exposed so a host can share one across several modules.
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The compiled module, for diagnostics.
    #[must_use]
    pub fn module(&self) -> &Module {
        &self.module
    }

    fn has(&self, name: &str) -> bool {
        let store = self.store.borrow();
        self.instance.get_func(&*store, name).is_some()
    }

    /// Call a guest function with up to three i32 arguments and one i32 result.
    fn invoke(&self, name: &str, args: &[Val]) -> Result<i32, WasmError> {
        let mut store = self.store.borrow_mut();
        store
            .set_fuel(DEFAULT_FUEL)
            .map_err(|err| WasmError::Guest(err.to_string()))?;
        let func = self
            .instance
            .get_func(&*store, name)
            .ok_or_else(|| WasmError::MissingExport(name.to_owned()))?;
        let mut out = [Val::I32(0)];
        func.call(&mut *store, args, &mut out)
            .map_err(|err| match err.as_trap_code() {
                Some(TrapCode::OutOfFuel) => WasmError::OutOfFuel,
                _ => WasmError::Guest(err.to_string()),
            })?;
        Ok(out[0].i32().unwrap_or(-1))
    }

    /// Write `bytes` into guest memory, returning `(ptr, len)`.
    fn upload(&self, bytes: &[u8]) -> Result<(i32, i32), WasmError> {
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(WasmError::TooLarge(bytes.len(), MAX_CONFIG_BYTES));
        }
        let len = i32::try_from(bytes.len())
            .map_err(|_| WasmError::TooLarge(bytes.len(), i32::MAX as usize))?;
        let ptr = self.invoke("alloc", &[Val::I32(len)])?;
        if ptr < 0 {
            return Err(WasmError::Guest(
                "alloc returned a negative pointer".to_owned(),
            ));
        }
        let mut store = self.store.borrow_mut();
        let memory = self
            .instance
            .get_memory(&*store, "memory")
            .ok_or_else(|| WasmError::MissingExport("memory".to_owned()))?;
        memory
            .write(&mut *store, ptr as usize, bytes)
            .map_err(|err| WasmError::Guest(err.to_string()))?;
        Ok((ptr, len))
    }

    fn release(&self, ptr: i32, len: i32) {
        // A guest that cannot free its own buffer is not a host failure; the fuel budget and
        // the module's own memory limit are what keep it honest.
        let _ = self.invoke("dealloc", &[Val::I32(ptr), Val::I32(len)]);
    }

    /// Read the result buffer the guest pointed at.
    fn result(&self) -> Result<Vec<u8>, WasmError> {
        let ptr = self.invoke("result_ptr", &[])?;
        let len = self.invoke("result_len", &[])?;
        if ptr < 0 || len < 0 {
            return Err(WasmError::Guest(
                "the result pointer or length is negative".to_owned(),
            ));
        }
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        if len > MAX_RESULT_BYTES {
            return Err(WasmError::TooLarge(len, MAX_RESULT_BYTES));
        }
        let mut buffer = vec![0u8; len];
        let store = self.store.borrow_mut();
        let memory = self
            .instance
            .get_memory(&*store, "memory")
            .ok_or_else(|| WasmError::MissingExport("memory".to_owned()))?;
        memory
            .read(&*store, ptr as usize, &mut buffer)
            .map_err(|err| WasmError::Guest(err.to_string()))?;
        Ok(buffer)
    }

    fn parse<T: serde::de::DeserializeOwned>(
        &self,
        bytes: &[u8],
        what: &'static str,
    ) -> Result<T, WasmError> {
        let text = std::str::from_utf8(bytes).map_err(|err| WasmError::BadPayload {
            what,
            message: err.to_string(),
        })?;
        ron::from_str(text).map_err(|err| WasmError::BadPayload {
            what,
            message: err.to_string(),
        })
    }

    fn into_core(error: WasmError) -> CoreError {
        CoreError::GameplayFailed {
            system: SYSTEM_ID.to_owned(),
            message: error.to_string(),
        }
    }
}

/// The payload a guest receives at `begin`.
#[derive(serde::Serialize)]
struct BeginPayload<'a> {
    config: Option<&'a ron::Value>,
    narrative: &'a NarrativeSnapshot,
}

impl GameplaySystem for WasmGameplay {
    fn id(&self) -> SystemId {
        SystemId::from(SYSTEM_ID)
    }

    fn begin(
        &mut self,
        config: Option<&ron::Value>,
        narrative: &NarrativeSnapshot,
    ) -> Result<(), CoreError> {
        let payload = ron::ser::to_string(&BeginPayload { config, narrative }).map_err(|err| {
            Self::into_core(WasmError::BadPayload {
                what: "begin payload",
                message: err.to_string(),
            })
        })?;
        let (ptr, len) = self.upload(payload.as_bytes()).map_err(Self::into_core)?;
        let status = self
            .invoke("begin", &[Val::I32(ptr), Val::I32(len)])
            .map_err(Self::into_core)?;
        self.release(ptr, len);
        if status != 0 {
            return Err(Self::into_core(WasmError::Guest(format!(
                "begin returned {status}"
            ))));
        }
        self.started = true;
        self.config = config.cloned();
        self.finished = None;
        Ok(())
    }

    fn step(&mut self, ctx: &mut GameplayContext<'_>) -> Option<OutcomeKey> {
        if let Some(done) = &self.finished {
            return Some(done.clone());
        }
        if !self.started {
            return None;
        }

        let inputs = ron::ser::to_string(&ctx.inputs).ok()?;
        let (ptr, len) = self.upload(inputs.as_bytes()).ok()?;
        let status = self
            .invoke(
                "step",
                &[
                    Val::I32(i32::try_from(ctx.dt_ms).unwrap_or(i32::MAX)),
                    Val::I32(ptr),
                    Val::I32(len),
                ],
            )
            .ok()?;
        self.release(ptr, len);

        if status != 1 {
            return None;
        }

        let bytes = self.result().ok()?;
        let key = String::from_utf8(bytes).ok()?;
        if key.is_empty() {
            return None;
        }
        let outcome = OutcomeKey::from(key);
        self.finished = Some(outcome.clone());
        Some(outcome)
    }

    fn cancel(&mut self) {
        if self.started {
            let _ = self.invoke("cancel", &[]);
        }
        self.started = false;
        self.config = None;
        self.finished = None;
    }

    fn view(&self) -> Option<GameplayView> {
        if !self.started || !self.has("view") {
            return None;
        }
        if self.invoke("view", &[]).ok()? != 1 {
            return None;
        }
        let bytes = self.result().ok()?;
        self.parse(&bytes, "view").ok()
    }

    fn save(&self) -> Result<Option<ron::Value>, CoreError> {
        if !self.started || !self.has("save") {
            return Ok(None);
        }
        let status = self.invoke("save", &[]).map_err(Self::into_core)?;
        if status != 1 {
            return Ok(None);
        }
        let bytes = self.result().map_err(Self::into_core)?;
        let text = String::from_utf8(bytes).map_err(|err| {
            Self::into_core(WasmError::BadPayload {
                what: "save state",
                message: err.to_string(),
            })
        })?;
        let value = ron::from_str(&text).map_err(|err| {
            Self::into_core(WasmError::BadPayload {
                what: "save state",
                message: err.to_string(),
            })
        })?;
        Ok(Some(value))
    }

    fn restore(&mut self, state: Option<&ron::Value>) -> Result<(), CoreError> {
        let Some(state) = state else {
            return Ok(());
        };
        if !self.has("restore") {
            return Err(Self::into_core(WasmError::MissingExport(
                "restore".to_owned(),
            )));
        }
        let payload = ron::ser::to_string(state).map_err(|err| {
            Self::into_core(WasmError::BadPayload {
                what: "save state",
                message: err.to_string(),
            })
        })?;
        let (ptr, len) = self.upload(payload.as_bytes()).map_err(Self::into_core)?;
        let status = self
            .invoke("restore", &[Val::I32(ptr), Val::I32(len)])
            .map_err(Self::into_core)?;
        self.release(ptr, len);
        if status != 0 {
            return Err(Self::into_core(WasmError::Guest(format!(
                "restore returned {status}"
            ))));
        }
        Ok(())
    }
}
