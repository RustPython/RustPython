#[cfg(all(not(unix), feature = "threading"))]
use super::FramePtr;
#[cfg(feature = "threading")]
use crate::builtins::PyBaseExceptionRef;
#[cfg(feature = "threading")]
use alloc::sync::Arc;

use crate::frame::Frame;
use crate::{AsObject, PyObject, VirtualMachine};
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
    /// Top of the owning thread's Python call stack, published for
    /// cross-thread readers (`sys._current_frames`, cross-thread `f_back`).
    /// The rest of the stack is reachable via each frame's `previous` pointer.
    /// Written lock-free on the hot push/pop path with relaxed ordering; every
    /// cross-thread read runs under stop-the-world, which parks the owning
    /// thread at a safepoint and supplies the happens-before edge, so the
    /// pointer and the frames it reaches are quiescent and alive at read time.
    #[cfg(unix)]
    pub top_frame: AtomicPtr<Frame>,
    /// Raw frame pointers, valid while the owning thread's call stack is active.
    /// Readers must hold the Mutex and convert to FrameRef inside the lock.
    /// Used on non-unix threading builds, which have no stop-the-world.
    #[cfg(not(unix))]
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
    /// QSBR state for deferred memory reclamation. On non-unix threading
    /// builds there is no attach/detach state machine, so the slot stays
    /// online from registration to thread exit; a thread blocked in native
    /// code simply delays reclamation until it runs again.
    pub(crate) qsbr: Arc<crate::object::qsbr::QsbrSlot>,
}

#[cfg(feature = "threading")]
pub type CurrentFrameSlot = Arc<ThreadSlot>;

thread_local! {
    pub(super) static VM_STACK: RefCell<Vec<NonNull<VirtualMachine>>> = Vec::with_capacity(1).into();

    /// Thread state created through the GILState-style C API.
    ///
    /// This is separate from the current VM stack: it only means "attached now",
    /// while this owns the per-thread VM that may be detached and re-attached.
    /// Despite the historical CPython "GILState" name, this does not model a
    /// GIL; it stores the VM used by that compatibility API.
    ///
    /// The Box keeps the VM address stable while VM_STACK holds a raw pointer to it.
    /// This matters when release_current_thread() moves the owner out of TLS and
    /// drops it while the VM is still current, so object destructors can still find
    /// their VM.
    #[cfg(feature = "threading")]
    static GILSTATE_VM: RefCell<Option<Box<ThreadedVirtualMachine>>> = const { RefCell::new(None) };

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

    /// Cached pointer to this thread's `ThreadSlot::top_frame`, so the hot
    /// push/pop path can publish the top frame with a single relaxed store and
    /// no `CURRENT_THREAD_SLOT` RefCell borrow. Null until the slot is
    /// initialized; the `Arc<ThreadSlot>` in `CURRENT_THREAD_SLOT` keeps the
    /// pointee alive until `cleanup_current_thread_frames` clears this.
    #[cfg(all(unix, feature = "threading"))]
    static CURRENT_TOP_FRAME_SLOT: Cell<*const AtomicPtr<Frame>> =
        const { Cell::new(core::ptr::null()) };

}

#[must_use]
pub fn current_vm_is_set() -> bool {
    VM_STACK.with(|vms| !vms.borrow().is_empty())
}

pub fn with_current_vm<R>(f: impl FnOnce(&VirtualMachine) -> R) -> R {
    VM_STACK.with(|vms| {
        let vm = vms
            .borrow()
            .last()
            .copied()
            .expect("call with_current_vm() but no current VM is attached");
        // SAFETY: entries in VM_STACK either borrow a VM for the dynamic
        // scope of a set_current_vm()/enter_vm() call or point at GILSTATE_VM.
        f(unsafe { vm.as_ref() })
    })
}

fn set_current_vm<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    VM_STACK.with(|vms| {
        vms.borrow_mut().push(vm.into());
        scopeguard::defer! {
            vms.borrow_mut().pop();
        }
        f()
    })
}

pub fn try_with_current_vm<R>(f: impl FnOnce(&VirtualMachine) -> R) -> Option<R> {
    VM_STACK.with(|vms| {
        let vm = vms.borrow().last().copied()?;
        // SAFETY: entries in VM_STACK either borrow a VM for the dynamic
        // scope of a set_current_vm()/enter_vm() call or point at GILSTATE_VM.
        Some(f(unsafe { vm.as_ref() }))
    })
}

pub fn enter_vm<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    // Outermost enter_vm: transition DETACHED → ATTACHED
    #[cfg(all(unix, feature = "threading"))]
    let was_outermost = !current_vm_is_set();

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
        if was_outermost {
            detach_thread();
        }
    }

    set_current_vm(vm, f)
}

/// RAII counterpart to `enter_vm`, for code that runs Python bytecode across
/// several statements interspersed with `&mut VirtualMachine` calls
/// (`VirtualMachine::initialize`), where a single closure-based `enter_vm`
/// scope cannot be expressed because the borrow checker won't let a closure
/// hold `&mut VirtualMachine` at the same time `enter_vm` reborrows it as
/// `&VirtualMachine`. Construction only needs a transient `&VirtualMachine`
/// borrow, so it can be dropped before subsequent `&mut` use.
///
/// Without this, code that runs Python bytecode before any `enter_vm` scope
/// exists would leave the thread not ATTACHED, making lock-free type cache
/// reads unsound.
#[must_use]
pub(crate) struct VmBootstrapGuard {
    #[cfg(all(unix, feature = "threading"))]
    was_outermost: bool,
}

impl VmBootstrapGuard {
    pub(crate) fn new(vm: &VirtualMachine) -> Self {
        // Outermost: transition DETACHED → ATTACHED
        #[cfg(all(unix, feature = "threading"))]
        let was_outermost = !current_vm_is_set();

        // Initialize thread slot for this thread if not already done
        #[cfg(feature = "threading")]
        init_thread_slot_if_needed(vm);

        #[cfg(all(unix, feature = "threading"))]
        if was_outermost {
            attach_thread(vm);
        }

        VM_STACK.with(|vms| vms.borrow_mut().push(vm.into()));

        Self {
            #[cfg(all(unix, feature = "threading"))]
            was_outermost,
        }
    }
}

impl Drop for VmBootstrapGuard {
    fn drop(&mut self) {
        VM_STACK.with(|vms| {
            vms.borrow_mut().pop();
        });

        // Outermost exit: transition ATTACHED → DETACHED
        #[cfg(all(unix, feature = "threading"))]
        if self.was_outermost {
            detach_thread();
        }
    }
}

#[cfg(feature = "threading")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CurrentVmAttachState {
    AlreadyAttached,
    Attached,
}

/// Attach the current native thread to a RustPython VM until
/// `release_current_thread()` is called.
#[cfg(feature = "threading")]
pub fn attach_current_thread(
    make_vm: impl FnOnce() -> ThreadedVirtualMachine,
) -> CurrentVmAttachState {
    if current_vm_is_set() {
        return CurrentVmAttachState::AlreadyAttached;
    }

    GILSTATE_VM.with(|gilstate_vm| {
        let mut gilstate_vm = gilstate_vm.borrow_mut();
        let threaded_vm = gilstate_vm.get_or_insert_with(|| Box::new(make_vm()));
        let vm = &threaded_vm.vm;

        vm.c_stack_soft_limit
            .set(VirtualMachine::calculate_c_stack_soft_limit());

        init_thread_slot_if_needed(vm);

        #[cfg(unix)]
        attach_thread(vm);

        VM_STACK.with(|vms| {
            debug_assert!(vms.borrow().is_empty());
            vms.borrow_mut().push(vm.into());
        });
    });

    CurrentVmAttachState::Attached
}

#[cfg(feature = "threading")]
pub fn release_current_thread(state: CurrentVmAttachState) {
    if state == CurrentVmAttachState::AlreadyAttached {
        return;
    }

    let gilstate_vm = GILSTATE_VM.with(|gilstate_vm| gilstate_vm.borrow_mut().take());
    drop(gilstate_vm);

    VM_STACK.with(|vms| {
        vms.borrow_mut()
            .pop()
            .expect("release_current_thread() called without an attached VM");
    });

    #[cfg(unix)]
    detach_thread();
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
                #[cfg(unix)]
                top_frame: AtomicPtr::new(core::ptr::null_mut()),
                #[cfg(not(unix))]
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
                qsbr: crate::object::qsbr::QSBR.register(),
            });
            registry.insert(thread_id, new_slot.clone());
            drop(registry);
            #[cfg(all(unix, feature = "threading"))]
            CURRENT_TOP_FRAME_SLOT.with(|c| c.set(&new_slot.top_frame));
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
                        crate::object::qsbr::QSBR.online(&s.qsbr);
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
    // A stop-the-world may have been requested while this thread was detached.
    // Honoring it here (rather than only at the next bytecode safepoint) keeps
    // a thread doing rapid allow_threads calls from re-attaching and running
    // past the requester forever, which would stall stop-the-world. Done
    // outside the CURRENT_THREAD_SLOT borrow above because suspend re-borrows
    // it. Safe against a concurrent start_the_world: suspend_if_needed only
    // parks while the request is still live and self-recovers otherwise.
    suspend_if_needed(&vm.state.stop_the_world);
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
                Ok(_) => {
                    crate::object::qsbr::QSBR.offline(&s.qsbr);
                }
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

/// Whether the QSBR subsystem asked this thread to pass a checkpoint.
/// A missed or racing read of this flag is harmless: the pending
/// retirement is still processed at the next checkpoint or by the GC
/// backstop.
#[cfg(feature = "threading")]
pub(crate) fn qsbr_break_requested() -> bool {
    CURRENT_THREAD_SLOT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|s| s.qsbr.requested.load(Ordering::Relaxed))
    })
}

/// Pass a QSBR checkpoint: the calling thread holds no borrowed cache
/// pointers here (instruction boundary), so mark it quiescent and try to
/// free retired allocations.
#[cfg(feature = "threading")]
pub(crate) fn qsbr_checkpoint() {
    use crate::object::qsbr::QSBR;
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            s.qsbr.requested.store(false, Ordering::Relaxed);
            QSBR.quiescent_state(&s.qsbr);
        }
    });
    QSBR.process();
}

/// Debug check: lock-free type-cache reads are only sound on threads that
/// are registered with QSBR and currently ATTACHED.
#[cfg(all(unix, feature = "threading", debug_assertions))]
pub(crate) fn debug_assert_current_thread_attached() {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            debug_assert_eq!(
                s.state.load(Ordering::Relaxed),
                THREAD_ATTACHED,
                "type cache read while thread not ATTACHED"
            );
        }
    });
}

/// Push a frame pointer onto the current thread's shared frame stack.
/// The pointed-to frame must remain alive until the matching pop.
///
/// Only used on non-unix threading builds; unix builds publish the top frame
/// through `set_current_frame` writing `ThreadSlot::top_frame`.
#[cfg(all(not(unix), feature = "threading"))]
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
#[cfg(all(not(unix), feature = "threading"))]
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
    // Publish the top frame for cross-thread readers. The relaxed store is
    // ordered by stop-the-world at read time (see `ThreadSlot::top_frame`).
    #[cfg(all(unix, feature = "threading"))]
    {
        let slot_top = CURRENT_TOP_FRAME_SLOT.with(Cell::get);
        if !slot_top.is_null() {
            // SAFETY: points to this thread's `ThreadSlot::top_frame`, kept
            // alive by the Arc in `CURRENT_THREAD_SLOT` for the thread's life.
            unsafe { (*slot_top).store(frame as *mut Frame, Ordering::Relaxed) };
        }
    }
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
    // Clear the cached top-frame pointer before dropping the slot Arc so no
    // later `set_current_frame` dereferences freed slot memory.
    #[cfg(all(unix, feature = "threading"))]
    CURRENT_TOP_FRAME_SLOT.with(|c| c.set(core::ptr::null()));
    CURRENT_THREAD_SLOT.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Reinitialize thread slot after fork. Called in child process.
/// Creates a fresh slot and registers it for the current thread,
/// preserving the current thread's frames from the signal-safe frame chain.
///
/// Precondition: `reinit_locks_after_fork()` has already reset all
/// VmState locks to unlocked.
#[cfg(feature = "threading")]
pub fn reinit_frame_slot_after_fork(vm: &VirtualMachine) {
    let current_ident = crate::stdlib::_thread::get_ident();
    // On non-unix, rebuild the shared frame stack (bottom-to-top) from the
    // current thread's frame chain, which walks top-to-bottom via `previous`.
    #[cfg(not(unix))]
    let current_frames: Vec<FramePtr> = {
        let mut current_frames = Vec::new();
        let mut cur = get_current_frame();
        while !cur.is_null() {
            // SAFETY: the forking thread's chain frames are alive.
            let py = unsafe { crate::Py::<Frame>::from_payload_ptr(cur) };
            current_frames.push(FramePtr(unsafe { NonNull::new_unchecked(py as *mut _) }));
            cur = unsafe { (*cur).previous_frame() };
        }
        current_frames.reverse();
        current_frames
    };
    let new_slot = Arc::new(ThreadSlot {
        // The surviving child thread keeps executing its current frame chain,
        // whose top is the signal-safe `get_current_frame()`.
        #[cfg(unix)]
        top_frame: AtomicPtr::new(get_current_frame() as *mut Frame),
        #[cfg(not(unix))]
        frames: parking_lot::Mutex::new(current_frames),
        exception: crate::PyAtomicRef::from(vm.topmost_exception()),
        #[cfg(unix)]
        state: core::sync::atomic::AtomicI32::new(THREAD_ATTACHED),
        #[cfg(unix)]
        stop_requested: core::sync::atomic::AtomicBool::new(false),
        #[cfg(unix)]
        thread: std::thread::current(),
        qsbr: crate::object::qsbr::QSBR.register(),
    });
    #[cfg(all(unix, feature = "threading"))]
    CURRENT_TOP_FRAME_SLOT.with(|c| c.set(&new_slot.top_frame));

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
        let interp = {
            let vms = vms.borrow();
            match vms.iter().copied().exactly_one() {
                Ok(x) => {
                    debug_assert!(vm_owns_obj(x));
                    x
                }
                Err(mut others) => others.find(|x| vm_owns_obj(*x))?,
            }
        };
        // SAFETY: all references in VM_STACK should be valid, and should not be changed or moved
        // at least until this function returns and the stack unwinds to an enter_vm() call
        let vm = unsafe { interp.as_ref() };
        Some(set_current_vm(vm, || f(vm)))
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
        F: Send + 'static + FnOnce(&Self) -> R,
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
            datastack: core::cell::UnsafeCell::new(crate::datastack::DataStack::new()),
            wasm_id: self.wasm_id.clone(),
            exceptions: RefCell::default(),
            import_func: self.import_func.clone(),
            importlib: self.importlib.clone(),
            profile_func: RefCell::new(global_profile.unwrap_or_else(|| self.ctx.none())),
            trace_func: RefCell::new(global_trace.unwrap_or_else(|| self.ctx.none())),
            use_tracing: Cell::new(use_tracing),
            tracing_depth: Cell::new(0),
            recursion_limit: self.recursion_limit.clone(),
            signal_handlers: core::cell::OnceCell::new(),
            signal_rx: None,
            repr_guards: RefCell::default(),
            state: self.state.clone(),
            initialized: self.initialized,
            recursion_depth: Cell::new(0),
            c_stack_soft_limit: Cell::new(Self::calculate_c_stack_soft_limit()),
            async_gen_firstiter: RefCell::new(None),
            async_gen_finalizer: RefCell::new(None),
            asyncio_running_loop: RefCell::new(None),
            asyncio_running_task: RefCell::new(None),
            callable_cache: self.callable_cache.clone(),
            audit_hooks: RefCell::new(vec![]),
        };
        ThreadedVirtualMachine { vm }
    }
}
