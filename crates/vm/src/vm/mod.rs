//! Implement virtual machine to run instructions.
//!
//! See also:
//!   <https://github.com/ProgVal/pythonvm-rust/blob/master/src/processor/mod.rs>

#[cfg(feature = "rustpython-compiler")]
mod compile;
mod context;
mod interpreter;
mod method;
#[cfg(feature = "rustpython-compiler")]
mod python_run;
mod setting;
pub mod thread;
mod vm_new;
mod vm_object;
mod vm_ops;

use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    builtins::{
        self, PyBaseExceptionRef, PyDict, PyDictRef, PyInt, PyList, PyModule, PyStr, PyStrInterned,
        PyStrRef, PyTypeRef, PyUtf8Str, PyUtf8StrInterned, PyWeak,
        code::PyCode,
        dict::{PyDictItems, PyDictKeys, PyDictValues},
        pystr::AsPyStr,
        tuple::PyTuple,
    },
    codecs::CodecsRegistry,
    common::{hash::HashSecret, lock::PyMutex, rc::PyRc},
    convert::ToPyObject,
    exceptions::types::PyBaseException,
    frame::{ExecutionResult, Frame, FrameRef},
    frozen::FrozenModule,
    function::{ArgMapping, FuncArgs, PySetterValue},
    import,
    protocol::PyIterIter,
    scope::Scope,
    signal, stdlib,
    warn::WarningsState,
};
use alloc::{borrow::Cow, collections::BTreeMap};
#[cfg(all(unix, feature = "threading"))]
use core::sync::atomic::AtomicI64;
use core::{
    cell::{Cell, OnceCell, RefCell},
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use crossbeam_utils::atomic::AtomicCell;
#[cfg(unix)]
use nix::{
    sys::signal::{SaFlags, SigAction, SigSet, Signal::SIGINT, kill, sigaction},
    unistd::getpid,
};
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
};

pub use context::Context;
pub use interpreter::{Interpreter, InterpreterBuilder};
pub(crate) use method::PyMethod;
pub use setting::{CheckHashPycsMode, Paths, PyConfig, Settings};

pub const MAX_MEMORY_SIZE: usize = isize::MAX as usize;

// Objects are live when they are on stack, or referenced by a name (for now)

/// Top level container of a python virtual machine. In theory you could
/// create more instances of this struct and have them operate fully isolated.
///
/// To construct this, please refer to the [`Interpreter`]
pub struct VirtualMachine {
    pub builtins: PyRef<PyModule>,
    pub sys_module: PyRef<PyModule>,
    pub ctx: PyRc<Context>,
    pub frames: RefCell<Vec<FramePtr>>,
    /// Thread-local data stack for bump-allocating frame-local data
    /// (localsplus arrays for non-generator frames).
    datastack: core::cell::UnsafeCell<crate::datastack::DataStack>,
    pub wasm_id: Option<String>,
    exceptions: RefCell<ExceptionStack>,
    pub import_func: PyObjectRef,
    pub(crate) importlib: PyObjectRef,
    pub profile_func: RefCell<PyObjectRef>,
    pub trace_func: RefCell<PyObjectRef>,
    pub use_tracing: Cell<bool>,
    pub recursion_limit: Cell<usize>,
    pub(crate) signal_handlers: OnceCell<Box<RefCell<[Option<PyObjectRef>; signal::NSIG]>>>,
    pub(crate) signal_rx: Option<signal::UserSignalReceiver>,
    pub repr_guards: RefCell<HashSet<usize>>,
    pub state: PyRc<PyGlobalState>,
    pub initialized: bool,
    recursion_depth: Cell<usize>,
    /// C stack soft limit for detecting stack overflow (like c_stack_soft_limit)
    #[cfg_attr(any(miri, target_env = "musl"), allow(dead_code))]
    c_stack_soft_limit: Cell<usize>,
    /// Async generator firstiter hook (per-thread, set via sys.set_asyncgen_hooks)
    pub async_gen_firstiter: RefCell<Option<PyObjectRef>>,
    /// Async generator finalizer hook (per-thread, set via sys.set_asyncgen_hooks)
    pub async_gen_finalizer: RefCell<Option<PyObjectRef>>,
    /// Current running asyncio event loop for this thread
    pub asyncio_running_loop: RefCell<Option<PyObjectRef>>,
    /// Current running asyncio task for this thread
    pub asyncio_running_task: RefCell<Option<PyObjectRef>>,
    pub(crate) callable_cache: CallableCache,
}

/// Non-owning frame pointer for the frames stack.
/// The pointed-to frame is kept alive by the caller of with_frame/resume_gen_frame.
#[derive(Copy, Clone)]
pub struct FramePtr(NonNull<Py<Frame>>);

impl FramePtr {
    /// # Safety
    /// The pointed-to frame must still be alive.
    pub unsafe fn as_ref(&self) -> &Py<Frame> {
        unsafe { self.0.as_ref() }
    }
}

// SAFETY: FramePtr is only stored in the VM's frames Vec while the corresponding
// FrameRef is alive on the call stack. The Vec is always empty when the VM moves between threads.
unsafe impl Send for FramePtr {}

#[derive(Debug)]
struct ExceptionStack {
    /// Linked list of handled-exception slots (`_PyErr_StackItem` chain).
    /// Bottom element is the thread's base slot; generator/coroutine resume
    /// pushes an additional slot.  Normal frame calls do **not** push/pop.
    stack: Vec<Option<PyBaseExceptionRef>>,
}

impl Default for ExceptionStack {
    fn default() -> Self {
        // Thread's base `_PyErr_StackItem` – always present.
        Self { stack: vec![None] }
    }
}

/// Stop-the-world state for fork safety. Before `fork()`, the requester
/// stops all other Python threads so they are not holding internal locks.
#[cfg(all(unix, feature = "threading"))]
pub struct StopTheWorldState {
    /// Fast-path flag checked in the bytecode loop (like `_PY_EVAL_PLEASE_STOP_BIT`)
    pub(crate) requested: AtomicBool,
    /// Whether the world is currently stopped (`stw->world_stopped`).
    world_stopped: AtomicBool,
    /// Ident of the thread that requested the stop (like `stw->requester`)
    requester: AtomicU64,
    /// Signaled by suspending threads when their state transitions to SUSPENDED
    notify_mutex: std::sync::Mutex<()>,
    notify_cv: std::sync::Condvar,
    /// Number of non-requester threads still expected to park for current stop request.
    thread_countdown: AtomicI64,
    /// Number of stop-the-world attempts.
    stats_stop_calls: AtomicU64,
    /// Most recent stop-the-world wait duration in ns.
    stats_last_wait_ns: AtomicU64,
    /// Total accumulated stop-the-world wait duration in ns.
    stats_total_wait_ns: AtomicU64,
    /// Max observed stop-the-world wait duration in ns.
    stats_max_wait_ns: AtomicU64,
    /// Number of poll-loop iterations spent waiting.
    stats_poll_loops: AtomicU64,
    /// Number of ATTACHED threads observed while polling.
    stats_attached_seen: AtomicU64,
    /// Number of DETACHED->SUSPENDED parks requested by requester.
    stats_forced_parks: AtomicU64,
    /// Number of suspend notifications from worker threads.
    stats_suspend_notifications: AtomicU64,
    /// Number of yield loops while attach waited on SUSPENDED->DETACHED.
    stats_attach_wait_yields: AtomicU64,
    /// Number of yield loops while suspend waited on SUSPENDED->DETACHED.
    stats_suspend_wait_yields: AtomicU64,
}

#[cfg(all(unix, feature = "threading"))]
#[derive(Debug, Clone, Copy)]
pub struct StopTheWorldStats {
    pub stop_calls: u64,
    pub last_wait_ns: u64,
    pub total_wait_ns: u64,
    pub max_wait_ns: u64,
    pub poll_loops: u64,
    pub attached_seen: u64,
    pub forced_parks: u64,
    pub suspend_notifications: u64,
    pub attach_wait_yields: u64,
    pub suspend_wait_yields: u64,
    pub world_stopped: bool,
}

#[cfg(all(unix, feature = "threading"))]
impl Default for StopTheWorldState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(unix, feature = "threading"))]
impl StopTheWorldState {
    pub const fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
            world_stopped: AtomicBool::new(false),
            requester: AtomicU64::new(0),
            notify_mutex: std::sync::Mutex::new(()),
            notify_cv: std::sync::Condvar::new(),
            thread_countdown: AtomicI64::new(0),
            stats_stop_calls: AtomicU64::new(0),
            stats_last_wait_ns: AtomicU64::new(0),
            stats_total_wait_ns: AtomicU64::new(0),
            stats_max_wait_ns: AtomicU64::new(0),
            stats_poll_loops: AtomicU64::new(0),
            stats_attached_seen: AtomicU64::new(0),
            stats_forced_parks: AtomicU64::new(0),
            stats_suspend_notifications: AtomicU64::new(0),
            stats_attach_wait_yields: AtomicU64::new(0),
            stats_suspend_wait_yields: AtomicU64::new(0),
        }
    }

    /// Wake the stop-the-world requester (called by each thread that suspends).
    pub(crate) fn notify_suspended(&self) {
        self.stats_suspend_notifications
            .fetch_add(1, Ordering::Relaxed);
        // Synchronize with requester wait loop to avoid lost wakeups.
        let _guard = self.notify_mutex.lock().unwrap();
        self.decrement_thread_countdown(1);
        self.notify_cv.notify_one();
    }

    #[inline]
    fn init_thread_countdown(&self, vm: &VirtualMachine) -> i64 {
        let requester = self.requester.load(Ordering::Relaxed);
        let registry = vm.state.thread_frames.lock();
        // Keep requested/count initialization serialized with thread-slot
        // registration (which also takes this lock), matching the
        // HEAD_LOCK-guarded stop-the-world bookkeeping.
        self.requested.store(true, Ordering::Release);
        let count = registry
            .keys()
            .filter(|&&thread_id| thread_id != requester)
            .count();
        let count = (count.min(i64::MAX as usize)) as i64;
        self.thread_countdown.store(count, Ordering::Release);
        count
    }

    #[inline]
    fn decrement_thread_countdown(&self, n: u64) {
        if n == 0 {
            return;
        }
        let n = (n.min(i64::MAX as u64)) as i64;
        let prev = self.thread_countdown.fetch_sub(n, Ordering::AcqRel);
        if prev <= n {
            // Clamp at 0 for safety in case of duplicate notifications.
            self.thread_countdown.store(0, Ordering::Release);
        }
    }

    /// Try to CAS detached threads directly to SUSPENDED and check whether
    /// stop countdown reached zero after parking detached threads.
    fn park_detached_threads(&self, vm: &VirtualMachine) -> bool {
        use thread::{THREAD_ATTACHED, THREAD_DETACHED, THREAD_SUSPENDED};
        let requester = self.requester.load(Ordering::Relaxed);
        let registry = vm.state.thread_frames.lock();
        let mut attached_seen = 0u64;
        let mut forced_parks = 0u64;
        for (&id, slot) in registry.iter() {
            if id == requester {
                continue;
            }
            let state = slot.state.load(Ordering::Relaxed);
            if state == THREAD_DETACHED {
                // CAS DETACHED → SUSPENDED (park without thread cooperation)
                match slot.state.compare_exchange(
                    THREAD_DETACHED,
                    THREAD_SUSPENDED,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        slot.stop_requested.store(false, Ordering::Release);
                        forced_parks = forced_parks.saturating_add(1);
                    }
                    Err(THREAD_ATTACHED) => {
                        // Set per-thread stop bit (_PY_EVAL_PLEASE_STOP_BIT).
                        slot.stop_requested.store(true, Ordering::Release);
                        // Raced with a thread re-attaching; it will self-suspend.
                        attached_seen = attached_seen.saturating_add(1);
                    }
                    Err(THREAD_DETACHED) => {
                        // Extremely unlikely race; next poll will handle it.
                    }
                    Err(THREAD_SUSPENDED) => {
                        slot.stop_requested.store(false, Ordering::Release);
                        // Another path parked it first.
                    }
                    Err(other) => {
                        debug_assert!(
                            false,
                            "unexpected thread state in park_detached_threads: {other}"
                        );
                    }
                }
            } else if state == THREAD_ATTACHED {
                // Set per-thread stop bit (_PY_EVAL_PLEASE_STOP_BIT).
                slot.stop_requested.store(true, Ordering::Release);
                // Thread is in bytecode — it will see `requested` and self-suspend
                attached_seen = attached_seen.saturating_add(1);
            }
            // THREAD_SUSPENDED → already parked
        }
        if attached_seen != 0 {
            self.stats_attached_seen
                .fetch_add(attached_seen, Ordering::Relaxed);
        }
        if forced_parks != 0 {
            self.decrement_thread_countdown(forced_parks);
            self.stats_forced_parks
                .fetch_add(forced_parks, Ordering::Relaxed);
        }
        forced_parks != 0 && self.thread_countdown.load(Ordering::Acquire) == 0
    }

    /// Stop all non-requester threads (`stop_the_world`).
    ///
    /// 1. Sets `requested`, marking the requester thread.
    /// 2. CAS detached threads to SUSPENDED.
    /// 3. Waits (polling with 1 ms condvar timeout) for attached threads
    ///    to self-suspend in `check_signals`.
    pub fn stop_the_world(&self, vm: &VirtualMachine) {
        let start = std::time::Instant::now();
        let requester_ident = crate::stdlib::_thread::get_ident();
        self.requester.store(requester_ident, Ordering::Relaxed);
        self.stats_stop_calls.fetch_add(1, Ordering::Relaxed);
        let initial_countdown = self.init_thread_countdown(vm);
        stw_trace(format_args!("stop begin requester={requester_ident}"));
        if initial_countdown == 0 {
            self.world_stopped.store(true, Ordering::Release);
            #[cfg(debug_assertions)]
            self.debug_assert_all_non_requester_suspended(vm);
            stw_trace(format_args!(
                "stop end requester={requester_ident} wait_ns=0 polls=0"
            ));
            return;
        }

        let mut polls = 0u64;
        loop {
            if self.park_detached_threads(vm) {
                break;
            }
            polls = polls.saturating_add(1);
            // Wait up to 1 ms for a thread to notify us it suspended.
            // Re-check under the wait mutex first to avoid a lost-wake race:
            // a thread may have suspended and notified right before we enter wait.
            let guard = self.notify_mutex.lock().unwrap();
            if self.thread_countdown.load(Ordering::Acquire) == 0 || self.park_detached_threads(vm)
            {
                drop(guard);
                break;
            }
            let _ = self
                .notify_cv
                .wait_timeout(guard, core::time::Duration::from_millis(1));
        }
        if polls != 0 {
            self.stats_poll_loops.fetch_add(polls, Ordering::Relaxed);
        }
        let wait_ns = start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        self.stats_last_wait_ns.store(wait_ns, Ordering::Relaxed);
        self.stats_total_wait_ns
            .fetch_add(wait_ns, Ordering::Relaxed);
        let mut prev_max = self.stats_max_wait_ns.load(Ordering::Relaxed);
        while wait_ns > prev_max {
            match self.stats_max_wait_ns.compare_exchange_weak(
                prev_max,
                wait_ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => prev_max = observed,
            }
        }
        self.world_stopped.store(true, Ordering::Release);
        #[cfg(debug_assertions)]
        self.debug_assert_all_non_requester_suspended(vm);
        stw_trace(format_args!(
            "stop end requester={requester_ident} wait_ns={wait_ns} polls={polls}"
        ));
    }

    /// Resume all suspended threads (`start_the_world`).
    pub fn start_the_world(&self, vm: &VirtualMachine) {
        use thread::{THREAD_DETACHED, THREAD_SUSPENDED};
        let requester = self.requester.load(Ordering::Relaxed);
        stw_trace(format_args!("start begin requester={requester}"));
        let registry = vm.state.thread_frames.lock();
        // Clear the request flag BEFORE waking threads. Otherwise a thread
        // returning from allow_threads → attach_thread could observe
        // `requested == true`, re-suspend itself, and stay parked forever.
        // Keep this write under the registry lock to serialize with new
        // thread-slot initialization.
        self.requested.store(false, Ordering::Release);
        self.world_stopped.store(false, Ordering::Release);
        for (&id, slot) in registry.iter() {
            if id == requester {
                continue;
            }
            slot.stop_requested.store(false, Ordering::Release);
            let state = slot.state.load(Ordering::Relaxed);
            debug_assert!(
                state == THREAD_SUSPENDED,
                "non-requester thread not suspended at start-the-world: id={id} state={state}"
            );
            if state == THREAD_SUSPENDED {
                slot.state.store(THREAD_DETACHED, Ordering::Release);
                slot.thread.unpark();
            }
        }
        drop(registry);
        self.thread_countdown.store(0, Ordering::Release);
        self.requester.store(0, Ordering::Relaxed);
        #[cfg(debug_assertions)]
        self.debug_assert_all_non_requester_detached(vm);
        stw_trace(format_args!("start end requester={requester}"));
    }

    /// Reset after fork in the child (only one thread alive).
    pub fn reset_after_fork(&self) {
        self.requested.store(false, Ordering::Relaxed);
        self.world_stopped.store(false, Ordering::Relaxed);
        self.requester.store(0, Ordering::Relaxed);
        self.thread_countdown.store(0, Ordering::Relaxed);
        stw_trace(format_args!("reset-after-fork"));
    }

    #[inline]
    pub(crate) fn requester_ident(&self) -> u64 {
        self.requester.load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn notify_thread_gone(&self) {
        let _guard = self.notify_mutex.lock().unwrap();
        self.decrement_thread_countdown(1);
        self.notify_cv.notify_one();
    }

    pub fn stats_snapshot(&self) -> StopTheWorldStats {
        StopTheWorldStats {
            stop_calls: self.stats_stop_calls.load(Ordering::Relaxed),
            last_wait_ns: self.stats_last_wait_ns.load(Ordering::Relaxed),
            total_wait_ns: self.stats_total_wait_ns.load(Ordering::Relaxed),
            max_wait_ns: self.stats_max_wait_ns.load(Ordering::Relaxed),
            poll_loops: self.stats_poll_loops.load(Ordering::Relaxed),
            attached_seen: self.stats_attached_seen.load(Ordering::Relaxed),
            forced_parks: self.stats_forced_parks.load(Ordering::Relaxed),
            suspend_notifications: self.stats_suspend_notifications.load(Ordering::Relaxed),
            attach_wait_yields: self.stats_attach_wait_yields.load(Ordering::Relaxed),
            suspend_wait_yields: self.stats_suspend_wait_yields.load(Ordering::Relaxed),
            world_stopped: self.world_stopped.load(Ordering::Relaxed),
        }
    }

    pub fn reset_stats(&self) {
        self.stats_stop_calls.store(0, Ordering::Relaxed);
        self.stats_last_wait_ns.store(0, Ordering::Relaxed);
        self.stats_total_wait_ns.store(0, Ordering::Relaxed);
        self.stats_max_wait_ns.store(0, Ordering::Relaxed);
        self.stats_poll_loops.store(0, Ordering::Relaxed);
        self.stats_attached_seen.store(0, Ordering::Relaxed);
        self.stats_forced_parks.store(0, Ordering::Relaxed);
        self.stats_suspend_notifications.store(0, Ordering::Relaxed);
        self.stats_attach_wait_yields.store(0, Ordering::Relaxed);
        self.stats_suspend_wait_yields.store(0, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn add_attach_wait_yields(&self, n: u64) {
        if n != 0 {
            self.stats_attach_wait_yields
                .fetch_add(n, Ordering::Relaxed);
        }
    }

    #[inline]
    pub(crate) fn add_suspend_wait_yields(&self, n: u64) {
        if n != 0 {
            self.stats_suspend_wait_yields
                .fetch_add(n, Ordering::Relaxed);
        }
    }

    #[cfg(debug_assertions)]
    fn debug_assert_all_non_requester_suspended(&self, vm: &VirtualMachine) {
        use thread::THREAD_SUSPENDED;
        let requester = self.requester.load(Ordering::Relaxed);
        let registry = vm.state.thread_frames.lock();
        for (&id, slot) in registry.iter() {
            if id == requester {
                continue;
            }
            let state = slot.state.load(Ordering::Relaxed);
            debug_assert!(
                state == THREAD_SUSPENDED,
                "non-requester thread not suspended during stop-the-world: id={id} state={state}"
            );
        }
    }

    #[cfg(debug_assertions)]
    fn debug_assert_all_non_requester_detached(&self, vm: &VirtualMachine) {
        use thread::THREAD_SUSPENDED;
        let requester = self.requester.load(Ordering::Relaxed);
        let registry = vm.state.thread_frames.lock();
        for (&id, slot) in registry.iter() {
            if id == requester {
                continue;
            }
            let state = slot.state.load(Ordering::Relaxed);
            debug_assert!(
                state != THREAD_SUSPENDED,
                "non-requester thread still suspended after start-the-world: id={id} state={state}"
            );
        }
    }
}

#[cfg(all(unix, feature = "threading"))]
pub(super) fn stw_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| crate::host_env::os::var_os("RUSTPYTHON_STW_TRACE").is_some())
}

#[cfg(all(unix, feature = "threading"))]
pub(super) fn stw_trace(msg: core::fmt::Arguments<'_>) {
    if stw_trace_enabled() {
        use core::fmt::Write as _;

        // Avoid stdio locking here: this path runs around fork where a child
        // may inherit a borrowed stderr lock and panic on eprintln!/stderr.
        struct FixedBuf {
            buf: [u8; 512],
            len: usize,
        }

        impl core::fmt::Write for FixedBuf {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                if self.len >= self.buf.len() {
                    return Ok(());
                }
                let remain = self.buf.len() - self.len;
                let src = s.as_bytes();
                let n = src.len().min(remain);
                self.buf[self.len..self.len + n].copy_from_slice(&src[..n]);
                self.len += n;
                Ok(())
            }
        }

        let mut out = FixedBuf {
            buf: [0u8; 512],
            len: 0,
        };
        let _ = writeln!(
            &mut out,
            "[rp-stw tid={}] {}",
            crate::stdlib::_thread::get_ident(),
            msg
        );
        unsafe {
            let _ = libc::write(libc::STDERR_FILENO, out.buf.as_ptr().cast(), out.len);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CallableCache {
    pub len: Option<PyObjectRef>,
    pub isinstance: Option<PyObjectRef>,
    pub list_append: Option<PyObjectRef>,
    pub builtin_all: Option<PyObjectRef>,
    pub builtin_any: Option<PyObjectRef>,
}

pub struct PyGlobalState {
    pub config: PyConfig,
    pub module_defs: BTreeMap<&'static str, &'static builtins::PyModuleDef>,
    pub frozen: HashMap<&'static str, FrozenModule, ahash::RandomState>,
    pub stacksize: AtomicCell<usize>,
    pub thread_count: AtomicCell<usize>,
    pub hash_secret: HashSecret,
    pub atexit_funcs: PyMutex<Vec<Box<(PyObjectRef, FuncArgs)>>>,
    pub codec_registry: CodecsRegistry,
    pub finalizing: AtomicBool,
    pub warnings: WarningsState,
    pub override_frozen_modules: AtomicCell<isize>,
    pub before_forkers: PyMutex<Vec<PyObjectRef>>,
    pub after_forkers_child: PyMutex<Vec<PyObjectRef>>,
    pub after_forkers_parent: PyMutex<Vec<PyObjectRef>>,
    pub int_max_str_digits: AtomicCell<usize>,
    pub switch_interval: AtomicCell<f64>,
    /// Global trace function for all threads (set by sys._settraceallthreads)
    pub global_trace_func: PyMutex<Option<PyObjectRef>>,
    /// Global profile function for all threads (set by sys._setprofileallthreads)
    pub global_profile_func: PyMutex<Option<PyObjectRef>>,
    /// Global type mutation/versioning mutex for CPython-style FT type operations.
    pub type_mutex: PyMutex<()>,
    /// Main thread identifier (pthread_self on Unix)
    #[cfg(feature = "threading")]
    pub main_thread_ident: AtomicCell<u64>,
    /// Registry of all threads' slots for sys._current_frames() and sys._current_exceptions()
    #[cfg(feature = "threading")]
    pub thread_frames: parking_lot::Mutex<HashMap<u64, stdlib::_thread::CurrentFrameSlot>>,
    /// Registry of all ThreadHandles for fork cleanup
    #[cfg(feature = "threading")]
    pub thread_handles: parking_lot::Mutex<Vec<stdlib::_thread::HandleEntry>>,
    /// Registry for non-daemon threads that need to be joined at shutdown
    #[cfg(feature = "threading")]
    pub shutdown_handles: parking_lot::Mutex<Vec<stdlib::_thread::ShutdownEntry>>,
    /// sys.monitoring state (tool names, events, callbacks)
    pub monitoring: PyMutex<stdlib::sys::monitoring::MonitoringState>,
    /// Fast-path mask: OR of all tools' events. 0 means no monitoring overhead.
    pub monitoring_events: stdlib::sys::monitoring::MonitoringEventsMask,
    /// Incremented on every monitoring state change. Code objects compare their
    /// local version against this to decide whether re-instrumentation is needed.
    pub instrumentation_version: AtomicU64,
    /// Stop-the-world state for pre-fork thread suspension
    #[cfg(all(unix, feature = "threading"))]
    pub stop_the_world: StopTheWorldState,
}

pub fn process_hash_secret_seed() -> u32 {
    use std::sync::OnceLock;
    static SEED: OnceLock<u32> = OnceLock::new();
    // os_random is expensive, but this is only ever called once
    *SEED.get_or_init(|| u32::from_ne_bytes(rustpython_common::rand::os_random()))
}

impl VirtualMachine {
    fn init_callable_cache(&mut self) -> PyResult<()> {
        self.callable_cache.len = Some(self.builtins.get_attr("len", self)?);
        self.callable_cache.isinstance = Some(self.builtins.get_attr("isinstance", self)?);
        let list_append = self
            .ctx
            .types
            .list_type
            .get_attr(self.ctx.intern_str("append"))
            .ok_or_else(|| self.new_runtime_error("failed to cache list.append".to_owned()))?;
        self.callable_cache.list_append = Some(list_append);
        self.callable_cache.builtin_all = Some(self.builtins.get_attr("all", self)?);
        self.callable_cache.builtin_any = Some(self.builtins.get_attr("any", self)?);
        Ok(())
    }

    /// Bump-allocate `size` bytes from the thread data stack.
    ///
    /// # Safety
    /// The returned pointer must be freed by calling `datastack_pop` in LIFO order.
    #[inline(always)]
    pub(crate) fn datastack_push(&self, size: usize) -> *mut u8 {
        unsafe { (*self.datastack.get()).push(size) }
    }

    /// Check whether the thread data stack currently has room for `size` bytes.
    #[inline(always)]
    pub(crate) fn datastack_has_space(&self, size: usize) -> bool {
        unsafe { (*self.datastack.get()).has_space(size) }
    }

    /// Pop a previous data stack allocation.
    ///
    /// # Safety
    /// `base` must be a pointer returned by `datastack_push` on this VM,
    /// and all allocations made after it must already have been popped.
    #[inline(always)]
    pub(crate) unsafe fn datastack_pop(&self, base: *mut u8) {
        unsafe { (*self.datastack.get()).pop(base) }
    }

    /// Temporarily detach the current thread (ATTACHED → DETACHED) while
    /// running `f`, then re-attach afterwards.  Allows `stop_the_world` to
    /// park this thread during blocking syscalls.
    ///
    /// Equivalent to CPython's `Py_BEGIN_ALLOW_THREADS` / `Py_END_ALLOW_THREADS`.
    #[inline]
    pub fn allow_threads<R>(&self, f: impl FnOnce() -> R) -> R {
        thread::allow_threads(self, f)
    }

    /// Check whether the current thread is the main thread.
    /// Mirrors `_Py_ThreadCanHandleSignals`.
    #[allow(dead_code)]
    pub(crate) fn is_main_thread(&self) -> bool {
        #[cfg(feature = "threading")]
        {
            crate::stdlib::_thread::get_ident() == self.state.main_thread_ident.load()
        }
        #[cfg(not(feature = "threading"))]
        {
            true
        }
    }

    /// Create a new `VirtualMachine` structure.
    pub(crate) fn new(ctx: PyRc<Context>, state: PyRc<PyGlobalState>) -> Self {
        flame_guard!("new VirtualMachine");

        // make a new module without access to the vm; doesn't
        // set __spec__, __loader__, etc. attributes
        let new_module = |def| {
            PyRef::new_ref(
                PyModule::from_def(def),
                ctx.types.module_type.to_owned(),
                Some(ctx.new_dict()),
            )
        };

        // Hard-core modules:
        let builtins = new_module(stdlib::builtins::module_def(&ctx));
        let sys_module = new_module(stdlib::sys::module_def(&ctx));

        let import_func = ctx.none();
        let importlib = ctx.none();
        let profile_func = RefCell::new(ctx.none());
        let trace_func = RefCell::new(ctx.none());
        let signal_handlers = OnceCell::from(signal::new_signal_handlers());

        let vm = Self {
            builtins,
            sys_module,
            ctx,
            frames: RefCell::new(vec![]),
            datastack: core::cell::UnsafeCell::new(crate::datastack::DataStack::new()),
            wasm_id: None,
            exceptions: RefCell::default(),
            import_func,
            importlib,
            profile_func,
            trace_func,
            use_tracing: Cell::new(false),
            recursion_limit: Cell::new(if cfg!(debug_assertions) { 256 } else { 1000 }),
            signal_handlers,
            signal_rx: None,
            repr_guards: RefCell::default(),
            state,
            initialized: false,
            recursion_depth: Cell::new(0),
            c_stack_soft_limit: Cell::new(Self::calculate_c_stack_soft_limit()),
            async_gen_firstiter: RefCell::new(None),
            async_gen_finalizer: RefCell::new(None),
            asyncio_running_loop: RefCell::new(None),
            asyncio_running_task: RefCell::new(None),
            callable_cache: CallableCache::default(),
        };

        if vm.state.hash_secret.hash_str("")
            != vm
                .ctx
                .interned_str("")
                .expect("empty str must be interned")
                .hash(&vm)
        {
            panic!("Interpreters in same process must share the hash seed");
        }

        vm.builtins.init_dict(
            vm.ctx.intern_str("builtins"),
            Some(vm.ctx.intern_str(stdlib::builtins::DOC.unwrap()).to_owned()),
            &vm,
        );
        vm.sys_module.init_dict(
            vm.ctx.intern_str("sys"),
            Some(vm.ctx.intern_str(stdlib::sys::DOC.unwrap()).to_owned()),
            &vm,
        );
        // let name = vm.sys_module.get_attr("__name__", &vm).unwrap();
        vm
    }

    /// set up the encodings search function
    /// init_importlib must be called before this call
    #[cfg(feature = "encodings")]
    fn import_encodings(&mut self) -> PyResult<()> {
        self.import("encodings", 0).map_err(|import_err| {
            let rustpythonpath_env = crate::host_env::os::var("RUSTPYTHONPATH").ok();
            let pythonpath_env = crate::host_env::os::var("PYTHONPATH").ok();
            let env_set = rustpythonpath_env.as_ref().is_some() || pythonpath_env.as_ref().is_some();
            let path_contains_env = self.state.config.paths.module_search_paths.iter().any(|s| {
                Some(s.as_str()) == rustpythonpath_env.as_deref() || Some(s.as_str()) == pythonpath_env.as_deref()
            });

            let guide_message = if cfg!(feature = "freeze-stdlib") {
                "`rustpython_pylib` may not be set while using `freeze-stdlib` feature. Try using `rustpython::InterpreterBuilder::init_stdlib` or manually call `builder.add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)` in `rustpython_vm::Interpreter::builder()`."
            } else if !env_set {
                "Neither RUSTPYTHONPATH nor PYTHONPATH is set. Try setting one of them to the stdlib directory."
            } else if path_contains_env {
                "RUSTPYTHONPATH or PYTHONPATH is set, but it doesn't contain the encodings library. If you are customizing the RustPython vm/interpreter, try adding the stdlib directory to the path. If you are developing the RustPython interpreter, it might be a bug during development."
            } else {
                "RUSTPYTHONPATH or PYTHONPATH is set, but it wasn't loaded to `PyConfig::paths::module_search_paths`. If you are going to customize the RustPython vm/interpreter, those environment variables are not loaded in the Settings struct by default. Please try creating a customized instance of the Settings struct. If you are developing the RustPython interpreter, it might be a bug during development."
            };

            let mut msg = format!(
                "RustPython could not import the encodings module. It usually means something went wrong. Please carefully read the following messages and follow the steps.\n\
                \n\
                {guide_message}");
            if !cfg!(feature = "freeze-stdlib") {
                msg += "\n\
                If you don't have access to a consistent external environment (e.g. targeting wasm, embedding \
                    rustpython in another application), try enabling the `freeze-stdlib` feature.\n\
                If this is intended and you want to exclude the encodings module from your interpreter, please remove the `encodings` feature from `rustpython-vm` crate.";
            }

            let err = self.new_runtime_error(msg);
            err.set___cause__(Some(import_err));
            err
        })?;
        Ok(())
    }

    fn import_ascii_utf8_encodings(&mut self) -> PyResult<()> {
        // Use the Python import machinery (FrozenImporter) so modules get
        // proper __spec__ and __loader__ attributes.
        self.import("codecs", 0)?;

        // Use dotted names when freeze-stdlib is enabled (modules come from Lib/encodings/),
        // otherwise use underscored names (modules come from core_modules/).
        let (ascii_module_name, utf8_module_name) = if cfg!(feature = "freeze-stdlib") {
            ("encodings.ascii", "encodings.utf_8")
        } else {
            ("encodings_ascii", "encodings_utf_8")
        };

        // Register ascii encoding
        // __import__("encodings.ascii") returns top-level "encodings", so
        // look up the actual submodule in sys.modules.
        self.import(ascii_module_name, 0)?;
        let sys_modules = self.sys_module.get_attr(identifier!(self, modules), self)?;
        let ascii_module = sys_modules.get_item(ascii_module_name, self)?;
        let getregentry = ascii_module.get_attr("getregentry", self)?;
        let codec_info = getregentry.call((), self)?;
        self.state
            .codec_registry
            .register_manual("ascii", codec_info.try_into_value(self)?)?;

        // Register utf-8 encoding (also as "utf8" alias since normalize_encoding_name
        // maps "utf-8" → "utf_8" but leaves "utf8" as-is)
        self.import(utf8_module_name, 0)?;
        let utf8_module = sys_modules.get_item(utf8_module_name, self)?;
        let getregentry = utf8_module.get_attr("getregentry", self)?;
        let codec_info = getregentry.call((), self)?;
        let utf8_codec: crate::codecs::PyCodec = codec_info.try_into_value(self)?;
        self.state
            .codec_registry
            .register_manual("utf-8", utf8_codec.clone())?;
        self.state
            .codec_registry
            .register_manual("utf8", utf8_codec)?;

        // Register latin-1 / iso8859-1 aliases needed very early for stdio
        // bootstrap (e.g. PYTHONIOENCODING=latin-1).
        if cfg!(feature = "freeze-stdlib") {
            self.import("encodings.latin_1", 0)?;
            let latin1_module = sys_modules.get_item("encodings.latin_1", self)?;
            let getregentry = latin1_module.get_attr("getregentry", self)?;
            let codec_info = getregentry.call((), self)?;
            let latin1_codec: crate::codecs::PyCodec = codec_info.try_into_value(self)?;
            for name in ["latin-1", "latin_1", "latin1", "iso8859-1", "iso8859_1"] {
                self.state
                    .codec_registry
                    .register_manual(name, latin1_codec.clone())?;
            }
        }
        Ok(())
    }

    fn initialize(&mut self) {
        flame_guard!("init VirtualMachine");

        if self.initialized {
            panic!("Double Initialize Error");
        }

        // Initialize main thread ident before any threading operations
        #[cfg(feature = "threading")]
        stdlib::_thread::init_main_thread_ident(self);

        stdlib::builtins::init_module(self, &self.builtins);
        let callable_cache_init = self.init_callable_cache();
        self.expect_pyresult(callable_cache_init, "failed to initialize callable cache");
        stdlib::sys::init_module(self, &self.sys_module, &self.builtins);
        self.expect_pyresult(
            stdlib::sys::set_bootstrap_stderr(self),
            "failed to initialize bootstrap stderr",
        );

        let mut essential_init = || -> PyResult {
            import::import_builtin(self, "_typing")?;
            #[cfg(all(not(target_arch = "wasm32"), feature = "host_env"))]
            import::import_builtin(self, "_signal")?;
            #[cfg(any(feature = "parser", feature = "compiler"))]
            import::import_builtin(self, "_ast")?;
            #[cfg(not(feature = "threading"))]
            import::import_frozen(self, "_thread")?;
            let importlib = import::init_importlib_base(self)?;
            self.import_ascii_utf8_encodings()?;

            {
                let io = import::import_builtin(self, "_io")?;

                // Full stdio: FileIO → BufferedWriter → TextIOWrapper
                #[cfg(all(feature = "host_env", feature = "stdio"))]
                let make_stdio = |name: &str, fd: i32, write: bool| -> PyResult<PyObjectRef> {
                    let buffered_stdio = self.state.config.settings.buffered_stdio;
                    let unbuffered = write && !buffered_stdio;
                    let buf = crate::stdlib::_io::open(
                        self.ctx.new_int(fd).into(),
                        Some(if write { "wb" } else { "rb" }),
                        crate::stdlib::_io::OpenArgs {
                            buffering: if unbuffered { 0 } else { -1 },
                            closefd: false,
                            ..Default::default()
                        },
                        self,
                    )?;
                    let raw = if unbuffered {
                        buf.clone()
                    } else {
                        buf.get_attr("raw", self)?
                    };
                    raw.set_attr("name", self.ctx.new_str(format!("<{name}>")), self)?;
                    let isatty = self.call_method(&raw, "isatty", ())?.is_true(self)?;
                    let write_through = !buffered_stdio;
                    let line_buffering = buffered_stdio && (isatty || fd == 2);

                    let newline = if cfg!(windows) { None } else { Some("\n") };
                    let encoding = self.state.config.settings.stdio_encoding.as_deref();
                    // stderr always uses backslashreplace (ignores stdio_errors)
                    let errors = if fd == 2 {
                        Some("backslashreplace")
                    } else {
                        self.state.config.settings.stdio_errors.as_deref().or(
                            if self.state.config.settings.stdio_encoding.is_some() {
                                Some("strict")
                            } else {
                                Some("surrogateescape")
                            },
                        )
                    };

                    let stdio = self.call_method(
                        &io,
                        "TextIOWrapper",
                        (
                            buf,
                            encoding,
                            errors,
                            newline,
                            line_buffering,
                            write_through,
                        ),
                    )?;
                    let mode = if write { "w" } else { "r" };
                    stdio.set_attr("mode", self.ctx.new_str(mode), self)?;
                    Ok::<_, self::PyBaseExceptionRef>(stdio)
                };

                // Sandbox stdio: lightweight wrapper using Rust's std::io directly
                #[cfg(all(not(feature = "host_env"), feature = "stdio"))]
                let make_stdio = |name: &str, fd: i32, write: bool| {
                    let mode = if write { "w" } else { "r" };
                    let stdio = stdlib::sys::SandboxStdio {
                        fd,
                        name: format!("<{name}>"),
                        mode: mode.to_owned(),
                    }
                    .into_ref(&self.ctx);
                    Ok(stdio.into())
                };

                // No stdio: set to None (embedding use case)
                #[cfg(not(feature = "stdio"))]
                let make_stdio = |_name: &str, _fd: i32, _write: bool| {
                    Ok(crate::builtins::PyNone.into_pyobject(self))
                };

                let set_stdio = |name, fd, write| {
                    let stdio: PyObjectRef = make_stdio(name, fd, write)?;
                    let dunder_name = self.ctx.intern_str(format!("__{name}__"));
                    self.sys_module.set_attr(
                        dunder_name, // e.g. __stdin__
                        stdio.clone(),
                        self,
                    )?;
                    self.sys_module.set_attr(name, stdio, self)?;
                    Ok(())
                };
                set_stdio("stdin", 0, false)?;
                set_stdio("stdout", 1, true)?;
                set_stdio("stderr", 2, true)?;

                let io_open = io.get_attr("open", self)?;
                self.builtins.set_attr("open", io_open, self)?;
            }

            Ok(importlib)
        };

        let res = essential_init();
        let importlib = self.expect_pyresult(res, "essential initialization failed");

        #[cfg(feature = "host_env")]
        if self.state.config.settings.allow_external_library
            && cfg!(feature = "rustpython-compiler")
            && let Err(e) = import::init_importlib_package(self, importlib)
        {
            eprintln!(
                "importlib initialization failed. This is critical for many complicated packages."
            );
            self.print_exception(e);
        }

        #[cfg(not(feature = "host_env"))]
        let _ = importlib;

        let _expect_stdlib = cfg!(feature = "freeze-stdlib")
            || !self.state.config.paths.module_search_paths.is_empty();

        #[cfg(feature = "encodings")]
        if _expect_stdlib {
            if let Err(e) = self.import_encodings() {
                eprintln!(
                    "encodings initialization failed. Only utf-8 encoding will be supported."
                );
                self.print_exception(e);
            }
        } else {
            // Here may not be the best place to give general `path_list` advice,
            // but bare rustpython_vm::VirtualMachine users skipped proper settings must hit here while properly setup vm never enters here.
            eprintln!(
                "feature `encodings` is enabled but `paths.module_search_paths` is empty. \
                Please add the library path to `settings.path_list`. If you intended to disable the entire standard library (including the `encodings` feature), please also make sure to disable the `encodings` feature.\n\
                Tip: You may also want to add `\"\"` to `settings.path_list` in order to enable importing from the current working directory."
            );
        }

        self.initialized = true;
    }

    /// Set the custom signal channel for the interpreter
    pub fn set_user_signal_channel(&mut self, signal_rx: signal::UserSignalReceiver) {
        self.signal_rx = Some(signal_rx);
    }

    /// Execute Python bytecode (`.pyc`) from an in-memory buffer.
    ///
    /// When the RustPython CLI is available, `.pyc` files are normally executed by
    /// invoking `rustpython <input>.pyc`. This method provides an alternative for
    /// environments where the binary is unavailable or file I/O is restricted
    /// (e.g. WASM).
    ///
    /// ## Preparing a `.pyc` file
    ///
    /// First, compile a Python source file into bytecode:
    ///
    /// ```sh
    /// # Generate a .pyc file
    /// $ rustpython -m py_compile <input>.py
    /// ```
    ///
    /// ## Running the bytecode
    ///
    /// Load the resulting `.pyc` file into memory and execute it using the VM:
    ///
    /// ```no_run
    /// use rustpython_vm::Interpreter;
    /// Interpreter::without_stdlib(Default::default()).enter(|vm| {
    ///     let bytes = std::fs::read("__pycache__/<input>.rustpython-314.pyc").unwrap();
    ///     let main_scope = vm.new_scope_with_main().unwrap();
    ///     vm.run_pyc_bytes(&bytes, main_scope);
    /// });
    /// ```
    pub fn run_pyc_bytes(&self, pyc_bytes: &[u8], scope: Scope) -> PyResult<()> {
        let code = PyCode::from_pyc(pyc_bytes, Some("<pyc_bytes>"), None, None, self)?;
        self.with_simple_run("<source>", |_module_dict| {
            self.run_code_obj(code, scope)?;
            Ok(())
        })
    }

    pub fn run_code_obj(&self, code: PyRef<PyCode>, scope: Scope) -> PyResult {
        use crate::builtins::{PyFunction, PyModule};

        // Create a function object for module code, similar to CPython's PyEval_EvalCode
        let func = PyFunction::new(code.clone(), scope.globals.clone(), self)?;
        let func_obj = func.into_ref(&self.ctx).into();

        // Extract builtins from globals["__builtins__"], like PyEval_EvalCode
        let builtins = match scope
            .globals
            .get_item_opt(identifier!(self, __builtins__), self)?
        {
            Some(b) => {
                if let Some(module) = b.downcast_ref::<PyModule>() {
                    module.dict().into()
                } else {
                    b
                }
            }
            None => self.builtins.dict().into(),
        };

        let frame =
            Frame::new(code, scope, builtins, &[], Some(func_obj), false, self).into_ref(&self.ctx);
        self.run_frame(frame)
    }

    #[cold]
    pub fn run_unraisable(&self, e: PyBaseExceptionRef, msg: Option<String>, object: PyObjectRef) {
        // During interpreter finalization, sys.unraisablehook may not be available,
        // but we still need to report exceptions (especially from atexit callbacks).
        // Write directly to stderr like PyErr_FormatUnraisable.
        if self.state.finalizing.load(Ordering::Acquire) {
            self.write_unraisable_to_stderr(&e, msg.as_deref(), &object);
            return;
        }

        let sys_module = self.import("sys", 0).unwrap();
        let unraisablehook = sys_module.get_attr("unraisablehook", self).unwrap();

        let exc_type = e.class().to_owned();
        let exc_traceback = e.__traceback__().to_pyobject(self); // TODO: actual traceback
        let exc_value = e.into();
        let args = stdlib::sys::UnraisableHookArgsData {
            exc_type,
            exc_value,
            exc_traceback,
            err_msg: self.new_pyobj(msg),
            object,
        };
        if let Err(e) = unraisablehook.call((args,), self) {
            println!("{}", e.as_object().repr(self).unwrap());
        }
    }

    /// Write unraisable exception to stderr during finalization.
    /// Similar to _PyErr_WriteUnraisableDefaultHook in CPython.
    fn write_unraisable_to_stderr(
        &self,
        e: &PyBaseExceptionRef,
        msg: Option<&str>,
        object: &PyObjectRef,
    ) {
        // Get stderr once and reuse it
        let stderr = crate::stdlib::sys::get_stderr(self).ok();

        let write_to_stderr = |s: &str, stderr: &Option<PyObjectRef>, vm: &VirtualMachine| {
            if let Some(stderr) = stderr {
                let _ = vm.call_method(stderr, "write", (s.to_owned(),));
            } else {
                eprint!("{}", s);
            }
        };

        let msg_str = if let Some(msg) = msg {
            format!("{msg}: ")
        } else {
            "Exception ignored in: ".to_owned()
        };
        write_to_stderr(&msg_str, &stderr, self);

        let repr_result = object.repr(self);
        let repr_wtf8 = repr_result
            .as_ref()
            .map_or("<object repr failed>".as_ref(), |s| s.as_wtf8());
        write_to_stderr(&format!("{repr_wtf8}\n"), &stderr, self);

        // Write exception type and message
        let exc_type_name = e.class().name();
        let msg = match e.as_object().str(self) {
            Ok(exc_str) if !exc_str.as_wtf8().is_empty() => {
                format!("{}: {}\n", exc_type_name, exc_str.as_wtf8())
            }
            _ => format!("{}\n", exc_type_name),
        };
        write_to_stderr(&msg, &stderr, self);

        // Flush stderr to ensure output is visible
        if let Some(ref stderr) = stderr {
            let _ = self.call_method(stderr, "flush", ());
        }
    }

    #[inline(always)]
    pub fn run_frame(&self, frame: FrameRef) -> PyResult {
        match self.with_frame(frame, |f| f.run(self))? {
            ExecutionResult::Return(value) => Ok(value),
            _ => panic!("Got unexpected result from function"),
        }
    }

    /// Run `run` with main scope.
    fn with_simple_run(
        &self,
        path: &str,
        run: impl FnOnce(&Py<PyDict>) -> PyResult<()>,
    ) -> PyResult<()> {
        let sys_modules = self.sys_module.get_attr(identifier!(self, modules), self)?;
        let main_module = sys_modules.get_item(identifier!(self, __main__), self)?;
        let module_dict = main_module.dict().expect("main module must have __dict__");

        // Track whether we set __file__ (for cleanup)
        let set_file_name = !module_dict.contains_key(identifier!(self, __file__), self);
        if set_file_name {
            module_dict.set_item(
                identifier!(self, __file__),
                self.ctx.new_str(path).into(),
                self,
            )?;
            module_dict.set_item(identifier!(self, __cached__), self.ctx.none(), self)?;
        }

        let result = run(&module_dict);

        self.flush_io();

        // Cleanup __file__ and __cached__ after execution
        if set_file_name {
            let _ = module_dict.del_item(identifier!(self, __file__), self);
            let _ = module_dict.del_item(identifier!(self, __cached__), self);
        }

        result
    }

    /// flush_io
    ///
    /// Flush stdout and stderr. Errors are silently ignored.
    fn flush_io(&self) {
        if let Ok(stdout) = self.sys_module.get_attr("stdout", self) {
            let _ = self.call_method(&stdout, identifier!(self, flush).as_str(), ());
        }
        if let Ok(stderr) = self.sys_module.get_attr("stderr", self) {
            let _ = self.call_method(&stderr, identifier!(self, flush).as_str(), ());
        }
    }

    /// Clear module references during shutdown.
    /// Follows the same phased algorithm as pylifecycle.c finalize_modules():
    /// no hardcoded module names, reverse import order, only builtins/sys last.
    pub fn finalize_modules(&self) {
        // Phase 1: Set special sys/builtins attributes to None, restore stdio
        self.finalize_modules_delete_special();

        // Phase 2: Remove all modules from sys.modules (set values to None),
        // and collect weakrefs to modules preserving import order.
        // No strong refs are kept — modules freed when their last ref drops.
        let module_weakrefs = self.finalize_remove_modules();

        // Phase 3: Clear sys.modules dict
        self.finalize_clear_modules_dict();

        // Phase 4: GC collect — modules removed from sys.modules are freed,
        // exposing cycles (e.g., dict ↔ function.__globals__). GC collects
        // these and calls __del__ while module dicts are still intact.
        crate::gc_state::gc_state().collect_force(2);

        // Phase 5: Clear module dicts in reverse import order using 2-pass algorithm.
        // Skip builtins and sys — those are cleared last.
        self.finalize_clear_module_dicts(&module_weakrefs);

        // Phase 6: GC collect — pick up anything freed by dict clearing.
        crate::gc_state::gc_state().collect_force(2);

        // Phase 7: Clear sys and builtins dicts last
        self.finalize_clear_sys_builtins_dict();
    }

    /// Phase 1: Set special sys attributes to None and restore stdio.
    fn finalize_modules_delete_special(&self) {
        let none = self.ctx.none();
        let sys_dict = self.sys_module.dict();

        // Set special sys attributes to None
        for attr in &[
            "path",
            "argv",
            "ps1",
            "ps2",
            "last_exc",
            "last_type",
            "last_value",
            "last_traceback",
            "path_importer_cache",
            "meta_path",
            "path_hooks",
        ] {
            let _ = sys_dict.set_item(*attr, none.clone(), self);
        }

        // Restore stdin/stdout/stderr from __stdin__/__stdout__/__stderr__
        for (std_name, dunder_name) in &[
            ("stdin", "__stdin__"),
            ("stdout", "__stdout__"),
            ("stderr", "__stderr__"),
        ] {
            let restored = sys_dict
                .get_item_opt(*dunder_name, self)
                .ok()
                .flatten()
                .unwrap_or_else(|| none.clone());
            let _ = sys_dict.set_item(*std_name, restored, self);
        }

        // builtins._ = None
        let _ = self.builtins.dict().set_item("_", none, self);
    }

    /// Phase 2: Set all sys.modules values to None and collect weakrefs.
    /// No strong refs are kept — modules are freed when removed from sys.modules
    /// (if nothing else references them), allowing GC to collect their cycles.
    fn finalize_remove_modules(&self) -> Vec<(String, PyRef<PyWeak>)> {
        let mut module_weakrefs = Vec::new();

        let Ok(modules) = self.sys_module.get_attr(identifier!(self, modules), self) else {
            return module_weakrefs;
        };
        let Some(modules_dict) = modules.downcast_ref::<PyDict>() else {
            return module_weakrefs;
        };

        let none = self.ctx.none();
        let items: Vec<_> = modules_dict.into_iter().collect();

        for (key, value) in items {
            let name = key
                .downcast_ref::<PyUtf8Str>()
                .map(|s| s.as_str().to_owned())
                .unwrap_or_default();

            // Save weakref to module (for later dict clearing)
            if value.downcast_ref::<PyModule>().is_some()
                && let Ok(weak) = value.downgrade(None, self)
            {
                module_weakrefs.push((name, weak));
            }

            // Set the value to None in sys.modules
            let _ = modules_dict.set_item(&*key, none.clone(), self);
        }

        module_weakrefs
    }

    /// Phase 3: Clear sys.modules dict.
    fn finalize_clear_modules_dict(&self) {
        if let Ok(modules) = self.sys_module.get_attr(identifier!(self, modules), self)
            && let Some(modules_dict) = modules.downcast_ref::<PyDict>()
        {
            modules_dict.clear();
        }
    }

    /// Phase 5: Clear module dicts in reverse import order.
    /// Skip builtins and sys — those are cleared last in Phase 7.
    fn finalize_clear_module_dicts(&self, module_weakrefs: &[(String, PyRef<PyWeak>)]) {
        let builtins_dict = self.builtins.dict();
        let sys_dict = self.sys_module.dict();

        for (_name, weakref) in module_weakrefs.iter().rev() {
            let Some(module_obj) = weakref.upgrade() else {
                continue;
            };
            let Some(module) = module_obj.downcast_ref::<PyModule>() else {
                continue;
            };

            let dict = module.dict();
            // Skip builtins and sys — they are cleared last
            if dict.is(&builtins_dict) || dict.is(&sys_dict) {
                continue;
            }

            Self::module_clear_dict(&dict, self);
        }
    }

    /// 2-pass module dict clearing (_PyModule_ClearDict algorithm).
    /// Pass 1: Set names starting with '_' (except __builtins__) to None.
    /// Pass 2: Set all remaining names (except __builtins__) to None.
    pub(crate) fn module_clear_dict(dict: &Py<PyDict>, vm: &VirtualMachine) {
        let none = vm.ctx.none();

        // Pass 1: names starting with '_' (except __builtins__)
        for (key, value) in dict.into_iter().collect::<Vec<_>>() {
            if vm.is_none(&value) {
                continue;
            }
            if let Some(key_str) = key.downcast_ref::<PyStr>() {
                let name = key_str.as_wtf8();
                if name.starts_with("_") && name != "__builtins__" {
                    let _ = dict.set_item(key_str, none.clone(), vm);
                }
            }
        }

        // Pass 2: all remaining (except __builtins__)
        for (key, value) in dict.into_iter().collect::<Vec<_>>() {
            if vm.is_none(&value) {
                continue;
            }
            if let Some(key_str) = key.downcast_ref::<PyStr>()
                && key_str.as_bytes() != b"__builtins__"
            {
                let _ = dict.set_item(key_str.as_wtf8(), none.clone(), vm);
            }
        }
    }

    /// Phase 7: Clear sys and builtins dicts last.
    fn finalize_clear_sys_builtins_dict(&self) {
        Self::module_clear_dict(&self.sys_module.dict(), self);
        Self::module_clear_dict(&self.builtins.dict(), self);
    }

    pub fn current_recursion_depth(&self) -> usize {
        self.recursion_depth.get()
    }

    /// Stack margin bytes (like _PyOS_STACK_MARGIN_BYTES).
    /// 2048 * sizeof(void*) = 16KB for 64-bit.
    #[cfg_attr(any(miri, target_env = "musl"), allow(dead_code))]
    const STACK_MARGIN_BYTES: usize = 2048 * core::mem::size_of::<usize>();

    /// Get the stack boundaries using platform-specific APIs.
    /// Returns (base, top) where base is the lowest address and top is the highest.
    #[cfg(all(not(miri), not(target_env = "musl"), windows))]
    fn get_stack_bounds() -> (usize, usize) {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThreadStackLimits, SetThreadStackGuarantee,
        };
        let mut low: usize = 0;
        let mut high: usize = 0;
        unsafe {
            GetCurrentThreadStackLimits(&mut low as *mut usize, &mut high as *mut usize);
            // Add the guaranteed stack space (reserved for exception handling)
            let mut guarantee: u32 = 0;
            SetThreadStackGuarantee(&mut guarantee);
            low += guarantee as usize;
        }
        (low, high)
    }

    /// Get stack boundaries on non-Windows platforms.
    /// Falls back to estimating based on current stack pointer.
    #[cfg(all(not(miri), not(target_env = "musl"), not(windows)))]
    fn get_stack_bounds() -> (usize, usize) {
        // Use pthread_attr_getstack on platforms that support it
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use libc::{
                pthread_attr_destroy, pthread_attr_getstack, pthread_attr_t, pthread_getattr_np,
                pthread_self,
            };
            let mut attr: pthread_attr_t = unsafe { core::mem::zeroed() };
            unsafe {
                if pthread_getattr_np(pthread_self(), &mut attr) == 0 {
                    let mut stack_addr: *mut libc::c_void = core::ptr::null_mut();
                    let mut stack_size: libc::size_t = 0;
                    if pthread_attr_getstack(&attr, &mut stack_addr, &mut stack_size) == 0 {
                        pthread_attr_destroy(&mut attr);
                        let base = stack_addr as usize;
                        let top = base + stack_size;
                        return (base, top);
                    }
                    pthread_attr_destroy(&mut attr);
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            use libc::{pthread_get_stackaddr_np, pthread_get_stacksize_np, pthread_self};
            unsafe {
                let thread = pthread_self();
                let stack_top = pthread_get_stackaddr_np(thread) as usize;
                let stack_size = pthread_get_stacksize_np(thread);
                let stack_base = stack_top - stack_size;
                return (stack_base, stack_top);
            }
        }

        // Fallback: estimate based on current SP and a default stack size
        #[allow(unreachable_code)]
        {
            let current_sp = psm::stack_pointer() as usize;
            // Assume 8MB stack, estimate base
            let estimated_size = 8 * 1024 * 1024;
            let base = current_sp.saturating_sub(estimated_size);
            let top = current_sp + 1024 * 1024; // Assume we're not at the very top
            (base, top)
        }
    }

    /// Calculate the C stack soft limit based on actual stack boundaries.
    /// soft_limit = base + 2 * margin (for downward-growing stacks)
    #[cfg(all(not(miri), not(target_env = "musl")))]
    fn calculate_c_stack_soft_limit() -> usize {
        let (base, _top) = Self::get_stack_bounds();
        base + Self::STACK_MARGIN_BYTES * 2
    }

    /// Musl currently reports stack bounds in a way that trips the VM's
    /// native stack guard during frozen stdlib bootstrap, so keep the Python
    /// recursion limit as the only guard there.
    #[cfg(any(miri, target_env = "musl"))]
    fn calculate_c_stack_soft_limit() -> usize {
        0
    }

    /// Check if we're near the C stack limit (like _Py_MakeRecCheck).
    /// Returns true only when stack pointer is in the "danger zone" between
    /// soft_limit and hard_limit (soft_limit - 2*margin).
    #[cfg(all(not(miri), not(target_env = "musl")))]
    #[inline(always)]
    fn check_c_stack_overflow(&self) -> bool {
        let current_sp = psm::stack_pointer() as usize;
        let soft_limit = self.c_stack_soft_limit.get();
        current_sp < soft_limit
            && current_sp >= soft_limit.saturating_sub(Self::STACK_MARGIN_BYTES * 2)
    }

    /// Miri does not support the native stack probe, and musl currently trips
    /// the probe during stdlib bootstrap.
    #[cfg(any(miri, target_env = "musl"))]
    #[inline(always)]
    fn check_c_stack_overflow(&self) -> bool {
        false
    }

    /// Used to run the body of a (possibly) recursive function. It will raise a
    /// RecursionError if recursive functions are nested far too many times,
    /// preventing a stack overflow.
    pub fn with_recursion<R, F: FnOnce() -> PyResult<R>>(&self, _where: &str, f: F) -> PyResult<R> {
        self.check_recursive_call(_where)?;

        // Native stack guard: check C stack like _Py_MakeRecCheck
        if self.check_c_stack_overflow() {
            return Err(self.new_recursion_error(_where.to_string()));
        }

        self.recursion_depth.update(|d| d + 1);
        scopeguard::defer! { self.recursion_depth.update(|d| d - 1) }
        f()
    }

    pub fn with_frame<R, F: FnOnce(FrameRef) -> PyResult<R>>(
        &self,
        frame: FrameRef,
        f: F,
    ) -> PyResult<R> {
        self.with_frame_impl(frame, true, f)
    }

    pub(crate) fn with_frame_untraced<R, F: FnOnce(FrameRef) -> PyResult<R>>(
        &self,
        frame: FrameRef,
        f: F,
    ) -> PyResult<R> {
        self.with_frame_impl(frame, false, f)
    }

    fn with_frame_impl<R, F: FnOnce(FrameRef) -> PyResult<R>>(
        &self,
        frame: FrameRef,
        traced: bool,
        f: F,
    ) -> PyResult<R> {
        self.with_recursion("", || {
            // SAFETY: `frame` (FrameRef) stays alive for the entire closure scope,
            // keeping the FramePtr valid. We pass a clone to `f` so that `f`
            // consuming its FrameRef doesn't invalidate our pointer.
            let fp = FramePtr(NonNull::from(&*frame));
            self.frames.borrow_mut().push(fp);
            // Update the shared frame stack for sys._current_frames() and faulthandler
            #[cfg(feature = "threading")]
            crate::vm::thread::push_thread_frame(fp);
            // Link frame into the signal-safe frame chain (previous pointer)
            let old_frame = crate::vm::thread::set_current_frame((&**frame) as *const Frame);
            frame.previous.store(
                old_frame as *mut Frame,
                core::sync::atomic::Ordering::Relaxed,
            );
            // Normal frame calls share the caller's exc_info slot so that
            // callees can see the caller's handled exception via sys.exc_info().
            // Save the current value to restore on exit — this prevents
            // exc_info pollution from frames with unbalanced
            // PUSH_EXC_INFO/POP_EXCEPT (e.g., exception escaping an except block
            // whose cleanup entry is missing from the exception table).
            let saved_exc = self.current_exception();
            let old_owner = frame.owner.swap(
                crate::frame::FrameOwner::Thread as i8,
                core::sync::atomic::Ordering::AcqRel,
            );

            // Ensure cleanup on panic: restore owner, exc_info, frame chain, and frames Vec.
            scopeguard::defer! {
                frame.owner.store(old_owner, core::sync::atomic::Ordering::Release);
                self.set_exception(saved_exc);
                crate::vm::thread::set_current_frame(old_frame);
                self.frames.borrow_mut().pop();
                #[cfg(feature = "threading")]
                crate::vm::thread::pop_thread_frame();
            }

            if traced {
                self.dispatch_traced_frame(&frame, |frame| f(frame.to_owned()))
            } else {
                f(frame.to_owned())
            }
        })
    }

    /// Frame execution for generator/coroutine resume.
    /// Pushes a new exc_info slot (gi_exc_state) onto the chain,
    /// linking the generator's saved handled-exception.
    pub fn resume_gen_frame<R, F: FnOnce(&Py<Frame>) -> PyResult<R>>(
        &self,
        frame: &FrameRef,
        exc: Option<PyBaseExceptionRef>,
        f: F,
    ) -> PyResult<R> {
        self.check_recursive_call("")?;
        if self.check_c_stack_overflow() {
            return Err(self.new_recursion_error(String::new()));
        }
        self.recursion_depth.update(|d| d + 1);

        // SAFETY: frame (&FrameRef) stays alive for the duration, so NonNull is valid until pop.
        let fp = FramePtr(NonNull::from(&**frame));
        self.frames.borrow_mut().push(fp);
        #[cfg(feature = "threading")]
        crate::vm::thread::push_thread_frame(fp);
        let old_frame = crate::vm::thread::set_current_frame((&***frame) as *const Frame);
        frame.previous.store(
            old_frame as *mut Frame,
            core::sync::atomic::Ordering::Relaxed,
        );
        // Push generator's exc_info slot onto the chain
        // (gi_exc_state.previous_item = tstate->exc_info;
        //  tstate->exc_info = &gi_exc_state;)
        self.push_exception(exc);
        let old_owner = frame.owner.swap(
            crate::frame::FrameOwner::Thread as i8,
            core::sync::atomic::Ordering::AcqRel,
        );

        // Ensure cleanup on panic: restore owner, pop exc_info slot, frame chain,
        // frames Vec, and recursion depth.
        scopeguard::defer! {
            frame.owner.store(old_owner, core::sync::atomic::Ordering::Release);
            self.pop_exception();
            crate::vm::thread::set_current_frame(old_frame);
            self.frames.borrow_mut().pop();
            #[cfg(feature = "threading")]
            crate::vm::thread::pop_thread_frame();

            self.recursion_depth.update(|d| d - 1);
        }

        self.dispatch_traced_frame(frame, |frame| f(frame))
    }

    /// Fire trace/profile 'call' and 'return' events around a frame body.
    ///
    /// Matches `call_trace_protected` / `trace_trampoline` protocol:
    /// - Fire `TraceEvent::Call`; if the trace function returns non-None,
    ///   install it as the per-frame `f_trace`.
    /// - Execute the closure (the actual frame body).
    /// - Fire `TraceEvent::Return` on both normal return **and** exception
    ///   unwind (`PY_UNWIND` → `PyTrace_RETURN` with `arg = None`).
    ///   Propagate any trace-function error, replacing the original exception.
    fn dispatch_traced_frame<R, F: FnOnce(&Py<Frame>) -> PyResult<R>>(
        &self,
        frame: &Py<Frame>,
        f: F,
    ) -> PyResult<R> {
        use crate::protocol::TraceEvent;

        // Fire 'call' trace event. current_frame() now returns the callee.
        let trace_result = self.trace_event(TraceEvent::Call, None)?;
        if let Some(local_trace) = trace_result {
            *frame.trace.lock() = local_trace;
        }

        let result = f(frame);

        // Fire 'return' event if frame is being traced or profiled.
        // PY_UNWIND fires PyTrace_RETURN with arg=None — so we fire for
        // both Ok and Err, matching `call_trace_protected` behavior.
        if self.use_tracing.get()
            && (!self.is_none(&frame.trace.lock()) || !self.is_none(&self.profile_func.borrow()))
        {
            let ret_result = self.trace_event(TraceEvent::Return, None);
            // call_trace_protected: if trace function raises, its error
            // replaces the original exception.
            ret_result?;
        }

        result
    }

    /// Returns a basic CompileOpts instance with options accurate to the vm. Used
    /// as the CompileOpts for `vm.compile()`.
    #[cfg(feature = "rustpython-codegen")]
    pub fn compile_opts(&self) -> crate::compiler::CompileOpts {
        crate::compiler::CompileOpts {
            optimize: self.state.config.settings.optimize,
            debug_ranges: self.state.config.settings.code_debug_ranges,
        }
    }

    // To be called right before raising the recursion depth.
    fn check_recursive_call(&self, _where: &str) -> PyResult<()> {
        if self.recursion_depth.get() >= self.recursion_limit.get() {
            Err(self.new_recursion_error(format!("maximum recursion depth exceeded {_where}")))
        } else {
            Ok(())
        }
    }

    pub fn current_frame(&self) -> Option<FrameRef> {
        self.frames.borrow().last().map(|fp| {
            // SAFETY: the caller keeps the FrameRef alive while it's in the Vec
            unsafe { fp.as_ref() }.to_owned()
        })
    }

    pub fn current_locals(&self) -> PyResult<ArgMapping> {
        self.current_frame()
            .expect("called current_locals but no frames on the stack")
            .locals(self)
    }

    pub fn current_globals(&self) -> PyDictRef {
        self.current_frame()
            .expect("called current_globals but no frames on the stack")
            .globals
            .clone()
    }

    pub fn try_class(&self, module: &'static str, class: &'static str) -> PyResult<PyTypeRef> {
        let class = self
            .import(module, 0)?
            .get_attr(class, self)?
            .downcast()
            .expect("not a class");
        Ok(class)
    }

    pub fn class(&self, module: &'static str, class: &'static str) -> PyTypeRef {
        let module = self
            .import(module, 0)
            .unwrap_or_else(|_| panic!("unable to import {module}"));

        let class = module
            .get_attr(class, self)
            .unwrap_or_else(|_| panic!("module {module:?} has no class {class}"));
        class.downcast().expect("not a class")
    }

    /// Call Python __import__ function without from_list.
    /// Roughly equivalent to `import module_name` or `import top.submodule`.
    ///
    /// See also [`VirtualMachine::import_from`] for more advanced import.
    /// See also [`rustpython_vm::import::import_source`] and other primitive import functions.
    #[inline]
    pub fn import<'a>(&self, module_name: impl AsPyStr<'a>, level: usize) -> PyResult {
        let module_name = module_name.as_pystr(&self.ctx);
        let from_list = self.ctx.empty_tuple_typed();
        self.import_inner(module_name, from_list, level)
    }

    /// Call Python __import__ function caller with from_list.
    /// Roughly equivalent to `from module_name import item1, item2` or `from top.submodule import item1, item2`
    #[inline]
    pub fn import_from<'a>(
        &self,
        module_name: impl AsPyStr<'a>,
        from_list: &Py<PyTuple<PyStrRef>>,
        level: usize,
    ) -> PyResult {
        let module_name = module_name.as_pystr(&self.ctx);
        self.import_inner(module_name, from_list, level)
    }

    fn import_inner(
        &self,
        module: &Py<PyStr>,
        from_list: &Py<PyTuple<PyStrRef>>,
        level: usize,
    ) -> PyResult {
        let import_func = self
            .builtins
            .get_attr(identifier!(self, __import__), self)
            .map_err(|_| self.new_import_error("__import__ not found", module.to_owned()))?;

        let (locals, globals) = if let Some(frame) = self.current_frame() {
            (
                Some(frame.locals.clone_mapping(self)),
                Some(frame.globals.clone()),
            )
        } else {
            (None, None)
        };
        let from_list: PyObjectRef = from_list.to_owned().into();
        import_func
            .call((module.to_owned(), globals, locals, from_list, level), self)
            .inspect_err(|exc| import::remove_importlib_frames(self, exc))
    }

    pub fn extract_elements_with<T, F>(&self, value: &PyObject, func: F) -> PyResult<Vec<T>>
    where
        F: Fn(PyObjectRef) -> PyResult<T>,
    {
        // Type-specific fast paths corresponding to _list_extend() in CPython
        // Objects/listobject.c. Each branch takes an atomic snapshot to avoid
        // race conditions from concurrent mutation (no GIL).
        let cls = value.class();
        let list_borrow;
        let slice = if cls.is(self.ctx.types.tuple_type) {
            value.downcast_ref::<PyTuple>().unwrap().as_slice()
        } else if cls.is(self.ctx.types.list_type) {
            list_borrow = value.downcast_ref::<PyList>().unwrap().borrow_vec();
            &list_borrow
        } else if cls.is(self.ctx.types.dict_type) {
            let keys = value.downcast_ref::<PyDict>().unwrap().keys_vec();
            return keys.into_iter().map(func).collect();
        } else if cls.is(self.ctx.types.dict_keys_type) {
            let keys = value.downcast_ref::<PyDictKeys>().unwrap().dict.keys_vec();
            return keys.into_iter().map(func).collect();
        } else if cls.is(self.ctx.types.dict_values_type) {
            let values = value
                .downcast_ref::<PyDictValues>()
                .unwrap()
                .dict
                .values_vec();
            return values.into_iter().map(func).collect();
        } else if cls.is(self.ctx.types.dict_items_type) {
            let items = value
                .downcast_ref::<PyDictItems>()
                .unwrap()
                .dict
                .items_vec();
            return items
                .into_iter()
                .map(|(k, v)| func(self.ctx.new_tuple(vec![k, v]).into()))
                .collect();
        } else {
            return self.map_py_iter(value, func);
        };
        slice.iter().map(|obj| func(obj.clone())).collect()
    }

    pub fn map_iterable_object<F, R>(&self, obj: &PyObject, mut f: F) -> PyResult<PyResult<Vec<R>>>
    where
        F: FnMut(PyObjectRef) -> PyResult<R>,
    {
        match_class!(match obj {
            ref l @ PyList => {
                let mut i: usize = 0;
                let mut results = Vec::with_capacity(l.borrow_vec().len());
                loop {
                    let elem = {
                        let elements = &*l.borrow_vec();
                        if i >= elements.len() {
                            results.shrink_to_fit();
                            return Ok(Ok(results));
                        } else {
                            elements[i].clone()
                        }
                        // free the lock
                    };
                    match f(elem) {
                        Ok(result) => results.push(result),
                        Err(err) => return Ok(Err(err)),
                    }
                    i += 1;
                }
            }
            ref t @ PyTuple => Ok(t.iter().cloned().map(f).collect()),
            // TODO: put internal iterable type
            obj => {
                Ok(self.map_py_iter(obj, f))
            }
        })
    }

    fn map_py_iter<F, R>(&self, value: &PyObject, mut f: F) -> PyResult<Vec<R>>
    where
        F: FnMut(PyObjectRef) -> PyResult<R>,
    {
        let iter = value.to_owned().get_iter(self)?;
        let cap = match self.length_hint_opt(value.to_owned()) {
            Err(e) if e.class().is(self.ctx.exceptions.runtime_error) => return Err(e),
            Ok(Some(value)) => Some(value),
            // Use a power of 2 as a default capacity.
            _ => None,
        };
        // TODO: fix extend to do this check (?), see test_extend in Lib/test/list_tests.py,
        // https://github.com/python/cpython/blob/v3.9.0/Objects/listobject.c#L922-L928
        if let Some(cap) = cap
            && cap >= isize::MAX as usize
        {
            return Ok(Vec::new());
        }

        let mut results = PyIterIter::new(self, iter.as_ref(), cap)
            .map(|element| f(element?))
            .collect::<PyResult<Vec<_>>>()?;
        results.shrink_to_fit();
        Ok(results)
    }

    pub fn get_attribute_opt<'a>(
        &self,
        obj: PyObjectRef,
        attr_name: impl AsPyStr<'a>,
    ) -> PyResult<Option<PyObjectRef>> {
        let attr_name = attr_name.as_pystr(&self.ctx);
        match obj.get_attr_inner(attr_name, self) {
            Ok(attr) => Ok(Some(attr)),
            Err(e) if e.fast_isinstance(self.ctx.exceptions.attribute_error) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_attribute_error_context(
        &self,
        exc: &Py<PyBaseException>,
        obj: PyObjectRef,
        name: PyStrRef,
    ) {
        if exc.class().is(self.ctx.exceptions.attribute_error) {
            let exc = exc.as_object();
            // Check if this exception was already augmented
            let already_set = exc
                .get_attr("name", self)
                .ok()
                .is_some_and(|v| !self.is_none(&v));
            if already_set {
                return;
            }
            exc.set_attr("name", name, self).unwrap();
            exc.set_attr("obj", obj, self).unwrap();
        }
    }

    // get_method should be used for internal access to magic methods (by-passing
    // the full getattribute look-up.
    pub fn get_method_or_type_error<F>(
        &self,
        obj: PyObjectRef,
        method_name: &'static PyStrInterned,
        err_msg: F,
    ) -> PyResult
    where
        F: FnOnce() -> String,
    {
        let method = obj
            .class()
            .get_attr(method_name)
            .ok_or_else(|| self.new_type_error(err_msg()))?;
        self.call_if_get_descriptor(&method, obj)
    }

    // TODO: remove + transfer over to get_special_method
    pub(crate) fn get_method(
        &self,
        obj: PyObjectRef,
        method_name: &'static PyStrInterned,
    ) -> Option<PyResult> {
        let method = obj.get_class_attr(method_name)?;
        Some(self.call_if_get_descriptor(&method, obj))
    }

    pub(crate) fn get_str_method(&self, obj: PyObjectRef, method_name: &str) -> Option<PyResult> {
        let method_name = self.ctx.interned_str(method_name)?;
        self.get_method(obj, method_name)
    }

    #[inline]
    pub(crate) fn eval_breaker_tripped(&self) -> bool {
        #[cfg(feature = "threading")]
        if self.state.finalizing.load(Ordering::Relaxed) && !self.is_main_thread() {
            return true;
        }

        #[cfg(all(unix, feature = "threading"))]
        if thread::stop_requested_for_current_thread() {
            return true;
        }

        #[cfg(not(target_arch = "wasm32"))]
        if crate::signal::is_triggered() {
            return true;
        }

        false
    }

    #[inline]
    /// Checks for triggered signals and calls the appropriate handlers. A no-op on
    /// platforms where signals are not supported.
    pub fn check_signals(&self) -> PyResult<()> {
        #[cfg(feature = "threading")]
        if self.state.finalizing.load(Ordering::Acquire) && !self.is_main_thread() {
            // once finalization starts,
            // non-main Python threads should stop running bytecode.
            return Err(self.new_exception(self.ctx.exceptions.system_exit.to_owned(), vec![]));
        }

        // Suspend this thread if stop-the-world is in progress
        #[cfg(all(unix, feature = "threading"))]
        thread::suspend_if_needed(&self.state.stop_the_world);

        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::signal::check_signals(self)
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(())
        }
    }

    /// Push a new exc_info slot (for generator/coroutine resume).
    pub(crate) fn push_exception(&self, exc: Option<PyBaseExceptionRef>) {
        self.exceptions.borrow_mut().stack.push(exc);
        #[cfg(feature = "threading")]
        thread::update_thread_exception(self.topmost_exception());
    }

    /// Pop the topmost exc_info slot (generator/coroutine yield/return).
    pub(crate) fn pop_exception(&self) -> Option<PyBaseExceptionRef> {
        let exc = self
            .exceptions
            .borrow_mut()
            .stack
            .pop()
            .expect("pop_exception() without nested exc stack");
        #[cfg(feature = "threading")]
        thread::update_thread_exception(self.topmost_exception());
        exc
    }

    pub(crate) fn current_exception(&self) -> Option<PyBaseExceptionRef> {
        self.exceptions.borrow().stack.last().cloned().flatten()
    }

    /// Set the current exc_info slot value (PUSH_EXC_INFO / POP_EXCEPT).
    pub(crate) fn set_exception(&self, exc: Option<PyBaseExceptionRef>) {
        // don't be holding the RefCell guard while __del__ is called
        let mut excs = self.exceptions.borrow_mut();
        debug_assert!(
            !excs.stack.is_empty(),
            "set_exception called with empty exception stack"
        );
        if let Some(top) = excs.stack.last_mut() {
            let prev = core::mem::replace(top, exc);
            drop(excs);
            drop(prev);
        } else {
            excs.stack.push(exc);
            drop(excs);
        }
        #[cfg(feature = "threading")]
        thread::update_thread_exception(self.topmost_exception());
    }

    pub(crate) fn contextualize_exception(&self, exception: &Py<PyBaseException>) {
        if let Some(context_exc) = self.topmost_exception()
            && !context_exc.is(exception)
        {
            // Traverse the context chain to find `exception` and break cycles
            // Uses Floyd's cycle detection: o moves every step, slow_o every other step
            let mut o = context_exc.clone();
            let mut slow_o = context_exc.clone();
            let mut slow_update_toggle = false;
            while let Some(context) = o.__context__() {
                if context.is(exception) {
                    o.set___context__(None);
                    break;
                }
                o = context;
                if o.is(&slow_o) {
                    // Pre-existing cycle detected - all exceptions on the path were visited
                    break;
                }
                if slow_update_toggle && let Some(slow_context) = slow_o.__context__() {
                    slow_o = slow_context;
                }
                slow_update_toggle = !slow_update_toggle;
            }
            exception.set___context__(Some(context_exc))
        }
    }

    pub(crate) fn topmost_exception(&self) -> Option<PyBaseExceptionRef> {
        let excs = self.exceptions.borrow();
        excs.stack.iter().rev().find_map(|e| e.clone())
    }

    pub fn handle_exit_exception(&self, exc: PyBaseExceptionRef) -> u32 {
        if exc.fast_isinstance(self.ctx.exceptions.system_exit) {
            let args = exc.args();
            let msg = match args.as_slice() {
                [] => return 0,
                [arg] => match_class!(match arg {
                    ref i @ PyInt => {
                        use num_traits::cast::ToPrimitive;
                        // Try u32 first, then i32 (for negative values), else -1 for overflow
                        let code = i
                            .as_bigint()
                            .to_u32()
                            .or_else(|| i.as_bigint().to_i32().map(|v| v as u32))
                            .unwrap_or(-1i32 as u32);
                        return code;
                    }
                    arg => {
                        if self.is_none(arg) {
                            return 0;
                        } else {
                            arg.str(self).ok()
                        }
                    }
                }),
                _ => args.as_object().repr(self).ok(),
            };
            if let Some(msg) = msg {
                // Write using Python's write() to use stderr's error handler (backslashreplace)
                if let Ok(stderr) = stdlib::sys::get_stderr(self) {
                    let _ = self.call_method(&stderr, "write", (msg,));
                    let _ = self.call_method(&stderr, "write", ("\n",));
                }
            }
            1
        } else if exc.fast_isinstance(self.ctx.exceptions.keyboard_interrupt) {
            #[allow(clippy::if_same_then_else)]
            {
                self.print_exception(exc);
                #[cfg(unix)]
                {
                    let action = SigAction::new(
                        nix::sys::signal::SigHandler::SigDfl,
                        SaFlags::SA_ONSTACK,
                        SigSet::empty(),
                    );
                    let result = unsafe { sigaction(SIGINT, &action) };
                    if result.is_ok() {
                        self.flush_std();
                        kill(getpid(), SIGINT).expect("Expect to be killed.");
                    }

                    (libc::SIGINT as u32) + 128
                }
                #[cfg(windows)]
                {
                    // STATUS_CONTROL_C_EXIT - same as CPython
                    0xC000013A
                }
                #[cfg(not(any(unix, windows)))]
                {
                    1
                }
            }
        } else {
            self.print_exception(exc);
            1
        }
    }

    #[doc(hidden)]
    pub fn __module_set_attr(
        &self,
        module: &Py<PyModule>,
        attr_name: &'static PyStrInterned,
        attr_value: impl Into<PyObjectRef>,
    ) -> PyResult<()> {
        let val = attr_value.into();
        module
            .as_object()
            .generic_setattr(attr_name, PySetterValue::Assign(val), self)
    }

    pub fn insert_sys_path(&self, obj: PyObjectRef) -> PyResult<()> {
        let sys_path = self.sys_module.get_attr("path", self).unwrap();
        self.call_method(&sys_path, "insert", (0, obj))?;
        Ok(())
    }

    pub fn run_module(&self, module: &str) -> PyResult<()> {
        let runpy = self.import("runpy", 0)?;
        let run_module_as_main = runpy.get_attr("_run_module_as_main", self)?;
        run_module_as_main.call((module,), self)?;
        Ok(())
    }

    pub fn fs_encoding(&self) -> &'static PyStrInterned {
        identifier!(self, utf_8)
    }

    pub fn fs_encode_errors(&self) -> &'static PyUtf8StrInterned {
        if cfg!(windows) {
            identifier_utf8!(self, surrogatepass)
        } else {
            identifier_utf8!(self, surrogateescape)
        }
    }

    pub fn fsdecode(&self, s: impl Into<OsString>) -> PyStrRef {
        match s.into().into_string() {
            Ok(s) => self.ctx.new_str(s),
            Err(s) => {
                let bytes = self.ctx.new_bytes(s.into_encoded_bytes());
                let errors = self.fs_encode_errors().to_owned();
                let res = self.state.codec_registry.decode_text(
                    bytes.into(),
                    "utf-8",
                    Some(errors),
                    self,
                );
                self.expect_pyresult(res, "fsdecode should be lossless and never fail")
            }
        }
    }

    pub fn fsencode<'a>(&self, s: &'a Py<PyStr>) -> PyResult<Cow<'a, OsStr>> {
        if cfg!(windows) || s.is_utf8() {
            // XXX: this is sketchy on windows; it's not guaranteed that the
            //      OsStr encoding will always be compatible with WTF-8.
            let s = unsafe { OsStr::from_encoded_bytes_unchecked(s.as_bytes()) };
            return Ok(Cow::Borrowed(s));
        }
        let errors = self.fs_encode_errors().to_owned();
        let bytes = self
            .state
            .codec_registry
            .encode_text(s.to_owned(), "utf-8", Some(errors), self)?
            .to_vec();
        // XXX: this is sketchy on windows; it's not guaranteed that the
        //      OsStr encoding will always be compatible with WTF-8.
        let s = unsafe { OsString::from_encoded_bytes_unchecked(bytes) };
        Ok(Cow::Owned(s))
    }
}

impl AsRef<Context> for VirtualMachine {
    fn as_ref(&self) -> &Context {
        &self.ctx
    }
}

/// Resolve frozen module alias to its original name.
/// Returns the original module name if an alias exists, otherwise returns the input name.
pub fn resolve_frozen_alias(name: &str) -> &str {
    match name {
        "_frozen_importlib" => "importlib._bootstrap",
        "_frozen_importlib_external" => "importlib._bootstrap_external",
        "encodings_ascii" => "encodings.ascii",
        "encodings_utf_8" => "encodings.utf_8",
        "__hello_alias__" | "__phello_alias__" | "__phello_alias__.spam" => "__hello__",
        "__phello__.__init__" => "<__phello__",
        "__phello__.ham.__init__" => "<__phello__.ham",
        "__hello_only__" => "",
        _ => name,
    }
}

#[test]
fn test_nested_frozen() {
    use rustpython_vm as vm;

    vm::Interpreter::builder(Default::default())
        .add_frozen_modules(rustpython_vm::py_freeze!(
            dir = "../../../../extra_tests/snippets"
        ))
        .build()
        .enter(|vm| {
            let scope = vm.new_scope_with_builtins();

            let source = "from dir_module.dir_module_inner import value2";
            let code_obj = vm
                .compile(source, vm::compiler::Mode::Exec, "<embedded>".to_owned())
                .map_err(|err| vm.new_syntax_error(&err, Some(source)))
                .unwrap();

            if let Err(e) = vm.run_code_obj(code_obj, scope) {
                vm.print_exception(e);
                panic!();
            }
        })
}

#[test]
fn frozen_origname_matches() {
    use rustpython_vm as vm;

    vm::Interpreter::builder(Default::default())
        .build()
        .enter(|vm| {
            let check = |name, expected| {
                let module = import::import_frozen(vm, name).unwrap();
                let origname: PyStrRef = module
                    .get_attr("__origname__", vm)
                    .unwrap()
                    .try_into_value(vm)
                    .unwrap();
                assert_eq!(origname.as_wtf8(), expected);
            };

            check("_frozen_importlib", "importlib._bootstrap");
            check(
                "_frozen_importlib_external",
                "importlib._bootstrap_external",
            );
        });
}
