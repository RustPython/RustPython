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

/// `main_id` value before any main interpreter has been registered.
const NO_MAIN_INTERPRETER: i64 = -1;

struct InterpreterRegistry {
    next_id: AtomicI64,
    /// Id of the first registered `is_main` interpreter (PEP 734 `get_main()`),
    /// or [`NO_MAIN_INTERPRETER`].
    main_id: AtomicI64,
    /// id → entry. Main interpreter is always id 0 when created first.
    entries: Mutex<HashMap<i64, RegistryEntry>>,
}

impl InterpreterRegistry {
    fn new() -> Self {
        Self {
            // Monotonic ids starting at 0. Concurrent Interpreter construction
            // (e.g. cargo test threads) must never share an id.
            next_id: AtomicI64::new(0),
            main_id: AtomicI64::new(NO_MAIN_INTERPRETER),
            entries: Mutex::new(HashMap::new()),
        }
    }
}

/// The interpreter registry.
///
/// With `threading` this is one process-global table. Without it, `PyRc` is
/// `Rc` and each OS thread owns an independent `Context::genesis()` and
/// `GcState`, so the registry is thread-local for the same reason `gc_state()`
/// is: an `Rc` handle must never be reachable from another thread.
/// `static_cell!` provides exactly that split.
fn registry() -> &'static InterpreterRegistry {
    rustpython_common::static_cell! {
        static REGISTRY: InterpreterRegistry;
    }
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

/// Id of the main interpreter (PEP 734 `get_main()`), or `None` before any
/// interpreter has been created.
///
/// This is distinct from [`PyGlobalState::is_main`]: every top-level (non-sub)
/// interpreter carries `is_main` for its own signal / main-thread bookkeeping,
/// but only the first one registered becomes *the* main.
#[must_use]
pub fn main_interpreter_id() -> Option<i64> {
    match registry().main_id.load(Ordering::Acquire) {
        NO_MAIN_INTERPRETER => None,
        id => Some(id),
    }
}

/// Allocate a unique interpreter id.
///
/// Ids are strictly monotonic and never reused for the lifetime of the
/// registry, so concurrent `Interpreter` construction (parallel unit tests,
/// multi-threaded embedding) never shares an id. Without `threading` the
/// registry — like `Context::genesis()` and the GC state — is per OS thread, so
/// ids are unique within a thread rather than across the process.
pub(crate) fn alloc_interpreter_id() -> i64 {
    registry().next_id.fetch_add(1, Ordering::Relaxed)
}

/// Gate between registering an interpreter and a collection's stop-the-world.
///
/// A collection snapshots the registry, stops every interpreter in the
/// snapshot, and then reads tracked objects with those threads parked. An
/// interpreter that registered after the snapshot was taken would not be in it,
/// so nothing would stop it, and its bootstrap — which runs Python and mutates
/// the shared generation lists — would run underneath that scan. Registration
/// therefore waits for an in-flight stop to end; the next collection's snapshot
/// then contains the new interpreter.
fn admission() -> &'static Mutex<()> {
    static ADMISSION: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    ADMISSION.get_or_init(|| Mutex::new(()))
}

/// Take the admission gate for the duration of a stop-the-world.
#[cfg(feature = "threading")]
pub(crate) fn lock_admission_for_stop() -> parking_lot::MutexGuard<'static, ()> {
    admission().lock()
}

/// Add the registry entry, behind the admission gate.
///
/// Only ever called with this thread detached, because the gate is held across
/// a stop-the-world: an attached thread waiting here, or re-attaching while
/// holding the gate, would leave that stop no safepoint to complete at. Nothing
/// under the gate blocks or allocates a tracked object, so this cannot re-enter
/// the collection it waits for.
fn insert_registry_entry(state: &PyRc<PyGlobalState>) {
    let _admission = admission().lock();
    let mut entries = registry().entries.lock();
    // Entries are weak and an interpreter's lifetime is decided by its last
    // `PyRc<PyGlobalState>` — which outlives the `Interpreter` handle whenever
    // `new_thread()` workers are still running — so nothing removes them at a
    // fixed point. Reap the dead ones here to bound the table instead.
    entries.retain(|_, entry| entry.state.strong_count() > 0);
    entries.insert(
        state.interpreter_id,
        RegistryEntry {
            whence: state.whence,
            state: PyRc::downgrade(state),
        },
    );
}

/// Register an interpreter state in the registry.
pub(crate) fn register_interpreter(state: &PyRc<PyGlobalState>) {
    let id = state.interpreter_id;
    if state.is_main {
        // First `is_main` interpreter defines the main for `get_main()`.
        // Additional top-level Interpreters (embedding) keep their own `is_main`
        // flag but do not displace the recorded main.
        let _ = registry().main_id.compare_exchange(
            NO_MAIN_INTERPRETER,
            id,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
    }
    // A subinterpreter is registered by a thread that is running its parent, so
    // detach for the whole insert rather than only for the wait.
    let detached = crate::vm::thread::try_with_current_vm(|vm| {
        vm.allow_threads(|| insert_registry_entry(state))
    });
    if detached.is_none() {
        insert_registry_entry(state);
    }
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

/// Reset the registry's locks after `fork()`.
///
/// The tables are reachable from every thread, so a thread that died in the
/// fork may have left one locked; the child would then deadlock the first time
/// it enumerates interpreters (which the collector now does on every stop).
///
/// # Safety
/// Must only be called after `fork()` in the child process, when no other
/// threads exist and the calling thread holds none of these locks.
#[cfg(all(unix, feature = "threading"))]
pub unsafe fn reinit_after_fork() {
    unsafe {
        crate::common::lock::reinit_mutex_after_fork(&registry().entries);
        crate::common::lock::reinit_mutex_after_fork(owned_interpreters());
        crate::common::lock::reinit_mutex_after_fork(admission());
    }
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
    use std::sync::OnceLock;
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
