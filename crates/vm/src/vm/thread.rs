#[cfg(feature = "threading")]
use super::FramePtr;
#[cfg(feature = "threading")]
use crate::builtins::PyBaseExceptionRef;
use crate::frame::Frame;
use crate::{AsObject, PyObject, VirtualMachine};
#[cfg(feature = "threading")]
use alloc::sync::Arc;
use core::{
    cell::{Cell, RefCell},
    ptr::NonNull,
    sync::atomic::{AtomicPtr, Ordering},
};
use itertools::Itertools;
use std::thread_local;

// Thread states for stop-the-world support.
//   DETACHED: not executing Python bytecode (in native code, or idle)
//   ATTACHED: actively executing Python bytecode
//   SUSPENDED: parked by a stop-the-world request
#[cfg(all(unix, feature = "threading"))]
pub const THREAD_DETACHED: i32 = 0;
#[cfg(all(unix, feature = "threading"))]
pub const THREAD_ATTACHED: i32 = 1;
#[cfg(all(unix, feature = "threading"))]
pub const THREAD_SUSPENDED: i32 = 2;

/// Per-thread shared state for sys._current_frames() and sys._current_exceptions().
/// The exception field uses atomic operations for lock-free cross-thread reads.
#[cfg(feature = "threading")]
pub struct ThreadSlot {
    /// Raw frame pointers, valid while the owning thread's call stack is active.
    /// Readers must hold the Mutex and convert to FrameRef inside the lock.
    pub frames: parking_lot::Mutex<Vec<FramePtr>>,
    pub exception: crate::PyAtomicRef<Option<crate::exceptions::types::PyBaseException>>,
    /// Thread state for stop-the-world: DETACHED / ATTACHED / SUSPENDED
    #[cfg(unix)]
    pub state: core::sync::atomic::AtomicI32,
    /// Per-thread stop request bit (eval breaker equivalent).
    #[cfg(unix)]
    pub stop_requested: core::sync::atomic::AtomicBool,
    /// Handle for waking this thread from park in stop-the-world paths.
    #[cfg(unix)]
    pub thread: std::thread::Thread,
}

#[cfg(feature = "threading")]
pub type CurrentFrameSlot = Arc<ThreadSlot>;

thread_local! {
    pub(super) static VM_STACK: RefCell<Vec<NonNull<VirtualMachine>>> = Vec::with_capacity(1).into();

    pub(crate) static COROUTINE_ORIGIN_TRACKING_DEPTH: Cell<u32> = const { Cell::new(0) };

    /// Current thread's slot for sys._current_frames() and sys._current_exceptions()
    #[cfg(feature = "threading")]
    static CURRENT_THREAD_SLOT: RefCell<Option<CurrentFrameSlot>> = const { RefCell::new(None) };

    /// Current top frame for signal-safe traceback walking.
    /// Mirrors `PyThreadState.current_frame`. Read by faulthandler's signal
    /// handler to dump tracebacks without accessing RefCell or locks.
    /// Uses AtomicPtr for async-signal-safety (signal handlers may read this
    /// while the owning thread is writing).
    pub(crate) static CURRENT_FRAME: AtomicPtr<Frame> =
        const { AtomicPtr::new(core::ptr::null_mut()) };

}

scoped_tls::scoped_thread_local!(pub static VM_CURRENT: VirtualMachine);

pub fn with_current_vm<R>(f: impl FnOnce(&VirtualMachine) -> R) -> R {
    if !VM_CURRENT.is_set() {
        panic!("call with_current_vm() but VM_CURRENT is null");
    }
    VM_CURRENT.with(f)
}

pub fn enter_vm<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    VM_STACK.with(|vms| {
        // Outermost enter_vm: transition DETACHED → ATTACHED
        #[cfg(all(unix, feature = "threading"))]
        let was_outermost = vms.borrow().is_empty();

        vms.borrow_mut().push(vm.into());

        // Initialize thread slot for this thread if not already done
        #[cfg(feature = "threading")]
        init_thread_slot_if_needed(vm);

        #[cfg(all(unix, feature = "threading"))]
        if was_outermost {
            attach_thread(vm);
        }

        scopeguard::defer! {
            // Outermost exit: transition ATTACHED → DETACHED
            #[cfg(all(unix, feature = "threading"))]
            if vms.borrow().len() == 1 {
                detach_thread();
            }
            vms.borrow_mut().pop();
        }
        VM_CURRENT.set(vm, f)
    })
}

/// Initialize thread slot for current thread if not already initialized.
/// Called automatically by enter_vm().
#[cfg(feature = "threading")]
fn init_thread_slot_if_needed(vm: &VirtualMachine) {
    CURRENT_THREAD_SLOT.with(|slot| {
        if slot.borrow().is_none() {
            let thread_id = crate::stdlib::_thread::get_ident();
            let mut registry = vm.state.thread_frames.lock();
            let new_slot = Arc::new(ThreadSlot {
                frames: parking_lot::Mutex::new(Vec::new()),
                exception: crate::PyAtomicRef::from(None::<PyBaseExceptionRef>),
                #[cfg(unix)]
                state: core::sync::atomic::AtomicI32::new(
                    if vm.state.stop_the_world.requested.load(Ordering::Acquire) {
                        // Match init_threadstate(): new thread-state starts
                        // suspended while stop-the-world is active.
                        THREAD_SUSPENDED
                    } else {
                        THREAD_DETACHED
                    },
                ),
                #[cfg(unix)]
                stop_requested: core::sync::atomic::AtomicBool::new(false),
                #[cfg(unix)]
                thread: std::thread::current(),
            });
            registry.insert(thread_id, new_slot.clone());
            drop(registry);
            *slot.borrow_mut() = Some(new_slot);
        }
    });
}

/// Transition DETACHED → ATTACHED. Blocks if the thread was SUSPENDED by
/// a stop-the-world request (like `_PyThreadState_Attach` + `tstate_wait_attach`).
#[cfg(all(unix, feature = "threading"))]
fn wait_while_suspended(slot: &ThreadSlot) -> u64 {
    let mut wait_yields = 0u64;
    while slot.state.load(Ordering::Acquire) == THREAD_SUSPENDED {
        wait_yields = wait_yields.saturating_add(1);
        std::thread::park();
    }
    wait_yields
}

#[cfg(all(unix, feature = "threading"))]
fn attach_thread(vm: &VirtualMachine) {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            super::stw_trace(format_args!("attach begin"));
            loop {
                match s.state.compare_exchange(
                    THREAD_DETACHED,
                    THREAD_ATTACHED,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        super::stw_trace(format_args!("attach DETACHED->ATTACHED"));
                        break;
                    }
                    Err(THREAD_SUSPENDED) => {
                        // Parked by stop-the-world — wait until released to DETACHED
                        super::stw_trace(format_args!("attach wait-suspended"));
                        let wait_yields = wait_while_suspended(s);
                        vm.state.stop_the_world.add_attach_wait_yields(wait_yields);
                        // Retry CAS
                    }
                    Err(state) => {
                        debug_assert!(false, "unexpected thread state in attach: {state}");
                        break;
                    }
                }
            }
        }
    });
}

/// Transition ATTACHED → DETACHED (like `_PyThreadState_Detach`).
#[cfg(all(unix, feature = "threading"))]
fn detach_thread() {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            match s.state.compare_exchange(
                THREAD_ATTACHED,
                THREAD_DETACHED,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {}
                Err(THREAD_DETACHED) => {
                    debug_assert!(false, "detach called while already DETACHED");
                    return;
                }
                Err(state) => {
                    debug_assert!(false, "unexpected thread state in detach: {state}");
                    return;
                }
            }
            super::stw_trace(format_args!("detach ATTACHED->DETACHED"));
        }
    });
}

/// Temporarily transition the current thread ATTACHED → DETACHED while
/// running `f`, then re-attach afterwards.  This allows `stop_the_world`
/// to park this thread during blocking operations.
///
/// `Py_BEGIN_ALLOW_THREADS` / `Py_END_ALLOW_THREADS` equivalent.
#[cfg(all(unix, feature = "threading"))]
pub fn allow_threads<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    // Preserve save/restore semantics:
    // only detach if this call observed ATTACHED at entry, and always restore
    // on unwind.
    let should_transition = CURRENT_THREAD_SLOT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|s| s.state.load(Ordering::Acquire) == THREAD_ATTACHED)
    });
    if !should_transition {
        return f();
    }

    detach_thread();
    let reattach_guard = scopeguard::guard(vm, attach_thread);
    let result = f();
    drop(reattach_guard);
    result
}

/// No-op on non-unix or non-threading builds.
#[cfg(not(all(unix, feature = "threading")))]
pub fn allow_threads<R>(_vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    f()
}

/// Called from check_signals when stop-the-world is requested.
/// Transitions ATTACHED → SUSPENDED and waits until released
/// (like `_PyThreadState_Suspend` + `_PyThreadState_Attach`).
#[cfg(all(unix, feature = "threading"))]
pub fn suspend_if_needed(stw: &super::StopTheWorldState) {
    let should_suspend = CURRENT_THREAD_SLOT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|s| s.stop_requested.load(Ordering::Relaxed))
    });
    if !should_suspend {
        return;
    }

    if !stw.requested.load(Ordering::Acquire) {
        CURRENT_THREAD_SLOT.with(|slot| {
            if let Some(s) = slot.borrow().as_ref() {
                s.stop_requested.store(false, Ordering::Release);
            }
        });
        return;
    }

    do_suspend(stw);
}

#[cfg(all(unix, feature = "threading"))]
#[cold]
fn do_suspend(stw: &super::StopTheWorldState) {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            // ATTACHED → SUSPENDED
            match s.state.compare_exchange(
                THREAD_ATTACHED,
                THREAD_SUSPENDED,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // Consumed this thread's stop request bit.
                    s.stop_requested.store(false, Ordering::Release);
                }
                Err(THREAD_DETACHED) => {
                    // Leaving VM; caller will re-check on next entry.
                    super::stw_trace(format_args!("suspend skip DETACHED"));
                    return;
                }
                Err(THREAD_SUSPENDED) => {
                    // Already parked by another path.
                    s.stop_requested.store(false, Ordering::Release);
                    super::stw_trace(format_args!("suspend skip already-suspended"));
                    return;
                }
                Err(state) => {
                    debug_assert!(false, "unexpected thread state in suspend: {state}");
                    return;
                }
            }
            super::stw_trace(format_args!("suspend ATTACHED->SUSPENDED"));

            // Re-check: if start_the_world already ran (cleared `requested`),
            // no one will set us back to DETACHED — we must self-recover.
            if !stw.requested.load(Ordering::Acquire) {
                s.state.store(THREAD_ATTACHED, Ordering::Release);
                s.stop_requested.store(false, Ordering::Release);
                super::stw_trace(format_args!("suspend abort requested-cleared"));
                return;
            }

            // Notify the stop-the-world requester that we've parked
            stw.notify_suspended();
            super::stw_trace(format_args!("suspend notified-requester"));

            // Wait until start_the_world sets us back to DETACHED
            let wait_yields = wait_while_suspended(s);
            stw.add_suspend_wait_yields(wait_yields);

            // Re-attach (DETACHED → ATTACHED), tstate_wait_attach CAS loop.
            loop {
                match s.state.compare_exchange(
                    THREAD_DETACHED,
                    THREAD_ATTACHED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(THREAD_SUSPENDED) => {
                        let extra_wait = wait_while_suspended(s);
                        stw.add_suspend_wait_yields(extra_wait);
                    }
                    Err(THREAD_ATTACHED) => break,
                    Err(state) => {
                        debug_assert!(false, "unexpected post-suspend state: {state}");
                        break;
                    }
                }
            }
            s.stop_requested.store(false, Ordering::Release);
            super::stw_trace(format_args!("suspend resume -> ATTACHED"));
        }
    });
}

#[cfg(all(unix, feature = "threading"))]
#[inline]
#[must_use]
pub fn stop_requested_for_current_thread() -> bool {
    CURRENT_THREAD_SLOT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|s| s.stop_requested.load(Ordering::Relaxed))
    })
}

/// Push a frame pointer onto the current thread's shared frame stack.
/// The pointed-to frame must remain alive until the matching pop.
#[cfg(feature = "threading")]
pub fn push_thread_frame(fp: FramePtr) {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            s.frames.lock().push(fp);
        } else {
            debug_assert!(
                false,
                "push_thread_frame called without initialized thread slot"
            );
        }
    });
}

/// Pop a frame from the current thread's shared frame stack.
/// Called when a frame is exited.
#[cfg(feature = "threading")]
pub fn pop_thread_frame() {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            s.frames.lock().pop();
        } else {
            debug_assert!(
                false,
                "pop_thread_frame called without initialized thread slot"
            );
        }
    });
}

/// Set the current thread's top frame pointer for signal-safe traceback walking.
/// Returns the previous frame pointer so it can be restored on pop.
pub fn set_current_frame(frame: *const Frame) -> *const Frame {
    CURRENT_FRAME.with(|c| c.swap(frame as *mut Frame, Ordering::Relaxed) as *const Frame)
}

/// Get the current thread's top frame pointer.
/// Used by faulthandler's signal handler to start traceback walking.
#[must_use]
pub fn get_current_frame() -> *const Frame {
    CURRENT_FRAME.with(|c| c.load(Ordering::Relaxed) as *const Frame)
}

/// Update the current thread's exception slot atomically (no locks).
/// Called from push_exception/pop_exception/set_exception.
#[cfg(feature = "threading")]
pub fn update_thread_exception(exc: Option<PyBaseExceptionRef>) {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            // SAFETY: Called only from the owning thread. The old ref is dropped
            // here on the owning thread, which is safe.
            let _old = unsafe { s.exception.swap(exc) };
        }
    });
}

/// Collect all threads' current exceptions for sys._current_exceptions().
/// Acquires the global registry lock briefly, then reads each slot's exception atomically.
#[cfg(feature = "threading")]
pub fn get_all_current_exceptions(vm: &VirtualMachine) -> Vec<(u64, Option<PyBaseExceptionRef>)> {
    let registry = vm.state.thread_frames.lock();
    registry
        .iter()
        .map(|(id, slot)| (*id, slot.exception.to_owned()))
        .collect()
}

/// Cleanup thread slot for the current thread. Called at thread exit.
#[cfg(feature = "threading")]
pub fn cleanup_current_thread_frames(vm: &VirtualMachine) {
    let thread_id = crate::stdlib::_thread::get_ident();
    let current_slot = CURRENT_THREAD_SLOT.with(|slot| slot.borrow().as_ref().cloned());

    // A dying thread should not remain logically ATTACHED while its
    // thread-state slot is being removed.
    #[cfg(all(unix, feature = "threading"))]
    if let Some(slot) = &current_slot {
        let _ = slot.state.compare_exchange(
            THREAD_ATTACHED,
            THREAD_DETACHED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    // Guard against OS thread-id reuse races: only remove the registry entry
    // if it still points at this thread's own slot.
    let _removed = if let Some(slot) = &current_slot {
        let mut registry = vm.state.thread_frames.lock();
        match registry.get(&thread_id) {
            Some(registered) if Arc::ptr_eq(registered, slot) => registry.remove(&thread_id),
            _ => None,
        }
    } else {
        None
    };
    #[cfg(all(unix, feature = "threading"))]
    if let Some(slot) = &_removed
        && vm.state.stop_the_world.requested.load(Ordering::Acquire)
        && thread_id != vm.state.stop_the_world.requester_ident()
        && slot.state.load(Ordering::Relaxed) != THREAD_SUSPENDED
    {
        // A non-requester thread disappeared while stop-the-world is pending.
        // Unblock requester countdown progress.
        vm.state.stop_the_world.notify_thread_gone();
    }
    CURRENT_THREAD_SLOT.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Reinitialize thread slot after fork. Called in child process.
/// Creates a fresh slot and registers it for the current thread,
/// preserving the current thread's frames from `vm.frames`.
///
/// Precondition: `reinit_locks_after_fork()` has already reset all
/// VmState locks to unlocked.
#[cfg(feature = "threading")]
pub fn reinit_frame_slot_after_fork(vm: &VirtualMachine) {
    let current_ident = crate::stdlib::_thread::get_ident();
    let current_frames: Vec<FramePtr> = vm.frames.borrow().clone();
    let new_slot = Arc::new(ThreadSlot {
        frames: parking_lot::Mutex::new(current_frames),
        exception: crate::PyAtomicRef::from(vm.topmost_exception()),
        #[cfg(unix)]
        state: core::sync::atomic::AtomicI32::new(THREAD_ATTACHED),
        #[cfg(unix)]
        stop_requested: core::sync::atomic::AtomicBool::new(false),
        #[cfg(unix)]
        thread: std::thread::current(),
    });

    // Lock is safe: reinit_locks_after_fork() already reset it to unlocked.
    let mut registry = vm.state.thread_frames.lock();
    registry.clear();
    registry.insert(current_ident, new_slot.clone());
    drop(registry);

    CURRENT_THREAD_SLOT.with(|s| {
        *s.borrow_mut() = Some(new_slot);
    });
}

pub fn with_vm<F, R>(obj: &PyObject, f: F) -> Option<R>
where
    F: Fn(&VirtualMachine) -> R,
{
    let vm_owns_obj = |interp: NonNull<VirtualMachine>| {
        // SAFETY: all references in VM_STACK should be valid
        let vm = unsafe { interp.as_ref() };
        obj.fast_isinstance(vm.ctx.types.object_type)
    };
    VM_STACK.with(|vms| {
        let interp = match vms.borrow().iter().copied().exactly_one() {
            Ok(x) => {
                debug_assert!(vm_owns_obj(x));
                x
            }
            Err(mut others) => others.find(|x| vm_owns_obj(*x))?,
        };
        // SAFETY: all references in VM_STACK should be valid, and should not be changed or moved
        // at least until this function returns and the stack unwinds to an enter_vm() call
        let vm = unsafe { interp.as_ref() };
        Some(VM_CURRENT.set(vm, || f(vm)))
    })
}

#[must_use = "ThreadedVirtualMachine does nothing unless you move it to another thread and call .run()"]
#[cfg(feature = "threading")]
pub struct ThreadedVirtualMachine {
    pub(super) vm: VirtualMachine,
}

#[cfg(feature = "threading")]
impl ThreadedVirtualMachine {
    /// Create a `FnOnce()` that can easily be passed to a function like [`std::thread::Builder::spawn`]
    ///
    /// # Note
    ///
    /// If you return a `PyObjectRef` (or a type that contains one) from `F`, and don't `join()`
    /// on the thread this `FnOnce` runs in, there is a possibility that that thread will panic
    /// as `PyObjectRef`'s `Drop` implementation tries to run the `__del__` destructor of a
    /// Python object but finds that it's not in the context of any vm.
    pub fn make_spawn_func<F, R>(self, f: F) -> impl FnOnce() -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        move || self.run(f)
    }

    /// Run a function in this thread context
    ///
    /// # Note
    ///
    /// If you return a `PyObjectRef` (or a type that contains one) from `F`, and don't return the object
    /// to the parent thread and then `join()` on the `JoinHandle` (or similar), there is a possibility that
    /// the current thread will panic as `PyObjectRef`'s `Drop` implementation tries to run the `__del__`
    /// destructor of a python object but finds that it's not in the context of any vm.
    pub fn run<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        let vm = &self.vm;
        // Each spawned thread has its own native stack bounds. Recompute the
        // soft limit here instead of inheriting the parent thread's value.
        vm.c_stack_soft_limit
            .set(VirtualMachine::calculate_c_stack_soft_limit());
        enter_vm(vm, || f(vm))
    }
}

impl VirtualMachine {
    /// Start a new thread with access to the same interpreter.
    ///
    /// # Note
    ///
    /// If you return a `PyObjectRef` (or a type that contains one) from `F`, and don't `join()`
    /// on the thread, there is a possibility that that thread will panic as `PyObjectRef`'s `Drop`
    /// implementation tries to run the `__del__` destructor of a python object but finds that it's
    /// not in the context of any vm.
    #[cfg(feature = "threading")]
    pub fn start_thread<F, R>(&self, f: F) -> std::thread::JoinHandle<R>
    where
        F: FnOnce(&Self) -> R,
        F: Send + 'static,
        R: Send + 'static,
    {
        let func = self.new_thread().make_spawn_func(f);
        std::thread::spawn(func)
    }

    /// Create a new VM thread that can be passed to a function like [`std::thread::spawn`]
    /// to use the same interpreter on a different thread. Note that if you just want to
    /// use this with `thread::spawn`, you can use
    /// [`vm.start_thread()`](`VirtualMachine::start_thread`) as a convenience.
    ///
    /// # Usage
    ///
    /// ```
    /// # rustpython_vm::Interpreter::without_stdlib(Default::default()).enter(|vm| {
    /// use std::thread::Builder;
    /// let handle = Builder::new()
    ///     .name("my thread :)".into())
    ///     .spawn(vm.new_thread().make_spawn_func(|vm| vm.ctx.none()))
    ///     .expect("couldn't spawn thread");
    /// let returned_obj = handle.join().expect("thread panicked");
    /// assert!(vm.is_none(&returned_obj));
    /// # })
    /// ```
    ///
    /// Note: this function is safe, but running the returned ThreadedVirtualMachine in the same
    /// thread context (i.e. with the same thread-local storage) doesn't have any
    /// specific guaranteed behavior.
    #[cfg(feature = "threading")]
    pub fn new_thread(&self) -> ThreadedVirtualMachine {
        let global_trace = self.state.global_trace_func.lock().clone();
        let global_profile = self.state.global_profile_func.lock().clone();
        let use_tracing = global_trace.is_some() || global_profile.is_some();

        let vm = Self {
            builtins: self.builtins.clone(),
            sys_module: self.sys_module.clone(),
            ctx: self.ctx.clone(),
            frames: RefCell::new(vec![]),
            datastack: core::cell::UnsafeCell::new(crate::datastack::DataStack::new()),
            wasm_id: self.wasm_id.clone(),
            exceptions: RefCell::default(),
            import_func: self.import_func.clone(),
            importlib: self.importlib.clone(),
            profile_func: RefCell::new(global_profile.unwrap_or_else(|| self.ctx.none())),
            trace_func: RefCell::new(global_trace.unwrap_or_else(|| self.ctx.none())),
            use_tracing: Cell::new(use_tracing),
            recursion_limit: self.recursion_limit.clone(),
            signal_handlers: core::cell::OnceCell::new(),
            signal_rx: None,
            repr_guards: RefCell::default(),
            state: self.state.clone(),
            initialized: self.initialized,
            recursion_depth: Cell::new(0),
            c_stack_soft_limit: Cell::new(VirtualMachine::calculate_c_stack_soft_limit()),
            async_gen_firstiter: RefCell::new(None),
            async_gen_finalizer: RefCell::new(None),
            asyncio_running_loop: RefCell::new(None),
            asyncio_running_task: RefCell::new(None),
            callable_cache: self.callable_cache.clone(),
        };
        ThreadedVirtualMachine { vm }
    }
}
