//! Process-global runtime support for multiple interpreters (PEP 734 preparation).
//!
//! CPython maps roughly as:
//! - this module ≈ `_PyRuntimeState.interpreters` + ID allocation
//! - [`crate::vm::PyGlobalState`] ≈ `PyInterpreterState`
//! - [`crate::VirtualMachine`] ≈ `PyThreadState` (plus shared refs to interpreter state)
//!
//! Multiple [`crate::Interpreter`] instances can coexist in one process. Each owns
//! an isolated `PyGlobalState` (modules, codecs, thread registry, stop-the-world, …)
//! while sharing the process-wide [`crate::Context`] (builtin types / immortals).

use crate::common::rc::PyRc;
use crate::vm::PyGlobalState;
use core::sync::atomic::{AtomicI64, Ordering};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Where an interpreter state came from (mirrors CPython `_PyInterpreterState_GetWhence`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum InterpreterWhence {
    /// Unknown / not recorded.
    Unknown = 0,
    /// Created as the process main interpreter at runtime init.
    Runtime = 1,
    /// Legacy C-API creation path (reserved for C-API parity).
    LegacyCapi = 2,
    /// Modern C-API creation path (reserved for C-API parity).
    Capi = 3,
    /// Cross-interpreter C-API (reserved).
    Xi = 4,
    /// Created via the stdlib / Rust subinterpreter API (PEP 734).
    Stdlib = 5,
}

impl InterpreterWhence {
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Snapshot of a registered interpreter for enumeration APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterpreterInfo {
    pub id: i64,
    pub whence: InterpreterWhence,
}

struct RegistryEntry {
    whence: InterpreterWhence,
    /// Weak handle so the registry does not keep interpreters alive.
    /// Type matches `PyRc` (Arc when threading, Rc otherwise).
    #[cfg(feature = "threading")]
    state: alloc::sync::Weak<PyGlobalState>,
    #[cfg(not(feature = "threading"))]
    state: alloc::rc::Weak<PyGlobalState>,
}

struct InterpreterRegistry {
    next_id: AtomicI64,
    /// id → entry. Main interpreter is always id 0 when created first.
    entries: Mutex<HashMap<i64, RegistryEntry>>,
}

// Without the `threading` feature `RegistryEntry` holds `rc::Weak`, which is
// `!Send`/`!Sync`, so the process-global registry static would not type-check.
// SAFETY: non-threading builds are single-threaded by construction (`Rc`-based
// objects are already unsound to touch across threads), and this process-global
// registry is only ever reached from that one thread. The `parking_lot::Mutex`
// still guards the map contents against reentrancy.
#[cfg(not(feature = "threading"))]
unsafe impl Send for InterpreterRegistry {}
#[cfg(not(feature = "threading"))]
unsafe impl Sync for InterpreterRegistry {}

impl InterpreterRegistry {
    fn new() -> Self {
        Self {
            // Monotonic ids starting at 0. Concurrent Interpreter construction
            // (e.g. cargo test threads) must never share an id.
            next_id: AtomicI64::new(0),
            entries: Mutex::new(HashMap::new()),
        }
    }
}

fn registry() -> &'static InterpreterRegistry {
    static REGISTRY: OnceLock<InterpreterRegistry> = OnceLock::new();
    REGISTRY.get_or_init(InterpreterRegistry::new)
}

/// Conventional id of the first process main interpreter when allocation is
/// sequential (CPython parity). Concurrent construction may assign other ids;
/// use [`PyGlobalState::is_main`] / [`crate::Interpreter::is_main`] to identify
/// a main interpreter, not this constant alone.
pub const MAIN_INTERPRETER_ID: i64 = 0;

/// Backs `sys.implementation.supports_isolated_interpreters`.
///
/// The Rust substrate already isolates interpreters (`PyGlobalState` per
/// interpreter, per-interpreter thread slots / stop-the-world). This stays
/// `false` until the Python-facing `_interpreters` module is wired up; flip it
/// in the commit that lands `_interpreters`.
pub const SUPPORTS_ISOLATED_INTERPRETERS: bool = false;

/// Process main interpreter id for PEP 734 `_interpreters.get_main()`, recorded
/// once when the first `is_main` interpreter is registered.
static MAIN_INTERPRETER: OnceLock<i64> = OnceLock::new();

/// Id of the process main interpreter (PEP 734 `get_main()`), or `None` before
/// any interpreter has been created.
///
/// This is distinct from [`PyGlobalState::is_main`]: every top-level (non-sub)
/// interpreter carries `is_main` for its own signal / main-thread bookkeeping,
/// but only the first one registered becomes *the* process main.
#[must_use]
pub fn main_interpreter_id() -> Option<i64> {
    MAIN_INTERPRETER.get().copied()
}

/// Allocate a unique process-global interpreter id.
///
/// Ids are strictly monotonic and never reused for the lifetime of the process.
/// This is required for thread-safe concurrent `Interpreter` construction
/// (parallel unit tests, multi-threaded embedding).
pub(crate) fn alloc_interpreter_id() -> i64 {
    registry().next_id.fetch_add(1, Ordering::Relaxed)
}

/// Register an interpreter state in the process-global table.
pub(crate) fn register_interpreter(state: &PyRc<PyGlobalState>) {
    let id = state.interpreter_id;
    let whence = state.whence;
    if state.is_main {
        // First `is_main` interpreter defines the process main for `get_main()`.
        // Additional top-level Interpreters (embedding) keep their own `is_main`
        // flag but do not displace the recorded process main.
        let _ = MAIN_INTERPRETER.set(id);
    }
    let mut entries = registry().entries.lock();
    entries.insert(
        id,
        RegistryEntry {
            whence,
            state: PyRc::downgrade(state),
        },
    );
}

/// Unregister an interpreter (called when its owning `Interpreter` is dropped).
pub(crate) fn unregister_interpreter(id: i64) {
    registry().entries.lock().remove(&id);
}

/// Look up a live interpreter state by id.
#[must_use]
pub fn lookup_interpreter(id: i64) -> Option<PyRc<PyGlobalState>> {
    let entries = registry().entries.lock();
    entries.get(&id).and_then(|e| e.state.upgrade())
}

/// List all currently registered (still-alive) interpreters.
#[must_use]
pub fn list_interpreters() -> Vec<InterpreterInfo> {
    let entries = registry().entries.lock();
    let mut out: Vec<InterpreterInfo> = entries
        .iter()
        .filter_map(|(&id, entry)| {
            // Drop dead weak refs from the listing.
            if entry.state.strong_count() == 0 {
                return None;
            }
            Some(InterpreterInfo {
                id,
                whence: entry.whence,
            })
        })
        .collect();
    out.sort_by_key(|info| info.id);
    out
}

/// Number of registered interpreters that are still alive.
#[must_use]
pub fn interpreter_count() -> usize {
    list_interpreters().len()
}

/// All live interpreter states, ordered by id.
///
/// Used by the cyclic collector, which must stop every interpreter's threads
/// (not just the collecting one) because GC-tracked objects from all
/// interpreters share one object graph. Ordering is deterministic so that
/// multiple stop-the-world requesters always take exclusions in the same order.
#[must_use]
pub fn live_interpreter_states() -> Vec<PyRc<PyGlobalState>> {
    let entries = registry().entries.lock();
    let mut states: Vec<(i64, PyRc<PyGlobalState>)> = entries
        .iter()
        .filter_map(|(&id, entry)| entry.state.upgrade().map(|state| (id, state)))
        .collect();
    drop(entries);
    states.sort_by_key(|(id, _)| *id);
    states.into_iter().map(|(_, state)| state).collect()
}

/// Runtime-owned interpreters (the ownership anchor for the Python
/// `_interpreters` API).
///
/// A Rust [`crate::Interpreter`] handle is normally owned by its Rust caller.
/// For PEP 734, `_interpreters.create()` returns only an id and the runtime
/// must keep the interpreter alive until `_interpreters.destroy(id)`. These
/// functions hold that ownership, keyed by interpreter id, while the weak
/// [`registry`] above still drives enumeration and lookup.
///
/// Only available with the `threading` feature: a runtime-owned interpreter is
/// reachable from other OS threads, which requires `Interpreter: Send` (true
/// only when `PyObjectRef` is `Arc`-backed).
#[cfg(feature = "threading")]
fn owned_interpreters() -> &'static Mutex<HashMap<i64, crate::Interpreter>> {
    static OWNED: OnceLock<Mutex<HashMap<i64, crate::Interpreter>>> = OnceLock::new();
    OWNED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Transfer ownership of `interp` to the runtime, returning its id.
#[cfg(feature = "threading")]
pub fn store_owned_interpreter(interp: crate::Interpreter) -> i64 {
    let id = interp.id();
    // Ids are strictly monotonic, so this never displaces (and drops) an
    // existing entry under the lock.
    owned_interpreters().lock().insert(id, interp);
    id
}

/// Reclaim a runtime-owned interpreter, removing it from the owner table.
///
/// The returned handle is dropped by the caller *outside* the owner lock; its
/// `Drop` unregisters the interpreter from the weak [`registry`].
#[cfg(feature = "threading")]
#[must_use]
pub fn take_owned_interpreter(id: i64) -> Option<crate::Interpreter> {
    owned_interpreters().lock().remove(&id)
}

/// Whether `id` refers to a runtime-owned interpreter.
#[cfg(feature = "threading")]
#[must_use]
pub fn is_owned_interpreter(id: i64) -> bool {
    owned_interpreters().lock().contains_key(&id)
}

/// Number of runtime-owned interpreters currently alive.
#[cfg(feature = "threading")]
#[must_use]
pub fn owned_interpreter_count() -> usize {
    owned_interpreters().lock().len()
}
