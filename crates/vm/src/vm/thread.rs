#[cfg(all(not(unix), feature = "threading"))]
use super::FramePtr;
#[cfg(feature = "threading")]
use crate::PyObjectRef;
#[cfg(feature = "threading")]
use crate::builtins::PyBaseExceptionRef;
#[cfg(feature = "threading")]
use alloc::sync::Arc;
#[cfg(feature = "threading")]
use rustpython_common::lock::PyMutex;

use crate::frame::InterpreterFrame;
use crate::vm::PyGlobalState;
#[cfg(all(unix, feature = "threading"))]
use crate::{Py, frame::FrameObject};
use crate::{PyObject, VirtualMachine};
#[cfg(all(unix, feature = "threading"))]
use core::sync::atomic::AtomicPtr;
use core::{
    cell::{Cell, RefCell},
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};
#[cfg(feature = "threading")]
use std::collections::HashMap;
use std::thread_local;

/// Thread states for stop-the-world support (`_Py_THREAD_*`).
///
/// DETACHED: not executing Python bytecode (in native code, or idle)
/// ATTACHED: actively executing Python bytecode
/// SUSPENDED: parked by a stop-the-world request
/// SHUTTING_DOWN: interpreter is finalizing; the OS thread must hang
/// (`_PyThreadState_HangThread`) and must not look done to `_ThreadHandle`.
#[cfg(feature = "threading")]
#[repr(i32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ThreadState {
    Detached = 0,
    Attached = 1,
    Suspended = 2,
    ShuttingDown = 3,
}

#[cfg(feature = "threading")]
impl ThreadState {
    #[must_use]
    pub const fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Detached),
            1 => Some(Self::Attached),
            2 => Some(Self::Suspended),
            3 => Some(Self::ShuttingDown),
            _ => None,
        }
    }
}

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
    pub top_frame: AtomicPtr<Py<FrameObject>>,
    /// Raw InterpreterFrame pointer, published alongside top_frame so
    /// cross-thread readers (sys._current_frames) can materialize
    /// stack-allocated frames that have no FrameObject.
    pub top_iframe: AtomicUsize,
    /// Raw frame pointers, valid while the owning thread's call stack is active.
    /// Readers must hold the Mutex and convert to FrameObjectRef inside the lock.
    /// Stands in for `top_frame` where that field is not built, so a reader
    /// that finds no `top_iframe` still has the frames to answer from.
    #[cfg(not(unix))]
    pub frames: parking_lot::Mutex<Vec<FramePtr>>,
    pub exception: crate::PyAtomicRef<Option<crate::exceptions::types::PyBaseException>>,
    /// `tstate->c_traceobj`. Cross-thread source of truth for sys.gettrace /
    /// sys._settraceallthreads.
    pub trace_func: PyMutex<PyObjectRef>,
    /// `tstate->c_profileobj`. Cross-thread source of truth for sys.getprofile /
    /// sys._setprofileallthreads.
    pub profile_func: PyMutex<PyObjectRef>,
    /// A retired slot may remain in native TLS, but must never retain Python
    /// callbacks again. Writers check this while holding the callback lock.
    closed: core::sync::atomic::AtomicBool,
    /// Thread state for stop-the-world: DETACHED / ATTACHED / SUSPENDED / SHUTTING_DOWN
    pub state: core::sync::atomic::AtomicI32,
    /// Per-thread stop request bit (eval breaker equivalent).
    pub stop_requested: core::sync::atomic::AtomicBool,
    /// Handle for waking this thread from park in stop-the-world paths.
    pub thread: std::thread::Thread,
    /// QSBR state for deferred memory reclamation.
    pub(crate) qsbr: Arc<crate::object::qsbr::QsbrSlot>,
    shared_qsbr: Arc<crate::object::qsbr::QsbrSlot>,
}

#[cfg(feature = "threading")]
pub type CurrentFrameSlot = Arc<ThreadSlot>;

#[cfg(feature = "threading")]
impl ThreadSlot {
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn replace_trace(&self, function: PyObjectRef) -> Option<PyObjectRef> {
        self.replace_callback(&self.trace_func, function)
    }

    pub(crate) fn replace_profile(&self, function: PyObjectRef) -> Option<PyObjectRef> {
        self.replace_callback(&self.profile_func, function)
    }

    fn replace_callback(
        &self,
        callback: &PyMutex<PyObjectRef>,
        function: PyObjectRef,
    ) -> Option<PyObjectRef> {
        let mut callback = callback.lock();
        if self.is_closed() {
            return None;
        }
        Some(core::mem::replace(&mut callback, function))
    }

    fn close(&self, vm: &VirtualMachine) {
        self.closed.store(true, Ordering::Release);
        // Swap everything before releasing any reference: destructors may call
        // sys.settrace() or sys._setprofileallthreads() reentrantly.
        let trace = core::mem::replace(&mut *self.trace_func.lock(), vm.ctx.none());
        let profile = core::mem::replace(&mut *self.profile_func.lock(), vm.ctx.none());
        // SAFETY: cleanup runs on the owner thread while still attached.
        let exception = unsafe { self.exception.swap(None) };
        drop((trace, profile, exception));
    }
}

/// Coalesced per-thread frame-publishing state, touched on every
/// `enter_iframe`/`exit_iframe`. Bundling `current_frame` together with
/// the cached `top_frame`/`top_iframe` slot pointers means
/// `set_current_frame` needs a single `thread_local!.with()` call
/// (one `_tlv_get_addr` on macOS) instead of two or three separate
/// ones — each `.with()` on a distinct `thread_local!` is its own TLS
/// lookup even though the bodies are just a cached-pointer store.
struct FrameSlotCache {
    /// Current top frame for signal-safe traceback walking.
    /// Stores a `*const InterpreterFrame` as `usize`.
    /// Read by faulthandler's signal handler to dump tracebacks without
    /// accessing RefCell or locks. Uses AtomicUsize for async-signal-safety.
    current_frame: AtomicUsize,
    /// Cached pointer to this thread's `ThreadSlot::top_frame`, so the hot
    /// push/pop path can publish the top frame with a single relaxed store and
    /// no `CURRENT_THREAD_SLOT` RefCell borrow. Null until the slot is
    /// initialized; the `Arc<ThreadSlot>` in `CURRENT_THREAD_SLOT` keeps the
    /// pointee alive until `cleanup_current_thread_frames` clears this.
    #[cfg(all(unix, feature = "threading"))]
    top_frame: Cell<*const AtomicPtr<Py<FrameObject>>>,
    /// Cached pointer to this thread's `ThreadSlot::top_iframe` for the hot
    /// light-frame push/pop path. The slot's Arc keeps the pointee alive.
    #[cfg(feature = "threading")]
    top_iframe: Cell<*const AtomicUsize>,
}

thread_local! {
    pub(super) static VM_STACK: RefCell<Vec<NonNull<VirtualMachine>>> = Vec::with_capacity(1).into();

    // The final destruction phase retains native control state, not a VM
    // pointer. Python callbacks and nested interpreter entry are unavailable.
    static NATIVE_TEARDOWN: RefCell<Option<crate::common::rc::PyRc<PyGlobalState>>> = const { RefCell::new(None) };
    static NATIVE_SWEEP: Cell<bool> = const { Cell::new(false) };

    // Native payloads may own another interpreter. Delay that destruction until
    // the outer teardown has restored attachment, and drain without recursion.
    #[expect(clippy::vec_box, reason = "queued VM owners retain stable addresses")]
    static DEFERRED_VMS: RefCell<Vec<Box<VirtualMachine>>> = const { RefCell::new(Vec::new()) };
    static DRAINING_VMS: Cell<bool> = const { Cell::new(false) };

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

    #[cfg(feature = "threading")]
    static GILSTATE_ENTRY: RefCell<Option<super::owner_lease::OwnerEntry>> = const { RefCell::new(None) };

    pub(crate) static COROUTINE_ORIGIN_TRACKING_DEPTH: Cell<u32> = const { Cell::new(0) };

    /// Per-interpreter thread slots for this OS thread (PEP 734 multi-interpreter).
    ///
    /// CPython keeps a `PyThreadState` per (thread, interpreter) pair. RustPython
    /// mirrors that: each interpreter's `PyGlobalState.thread_frames` gets its own
    /// [`ThreadSlot`] for this OS thread. `CURRENT_THREAD_SLOT` always points at
    /// the slot for the currently entered interpreter.
    #[cfg(feature = "threading")]
    static INTERP_THREAD_SLOTS: RefCell<HashMap<i64, CurrentFrameSlot>> =
        RefCell::new(HashMap::new());

    /// Current thread's slot for the currently entered interpreter.
    #[cfg(feature = "threading")]
    static CURRENT_THREAD_SLOT: RefCell<Option<CurrentFrameSlot>> = const { RefCell::new(None) };

    pub(crate) static FRAME_SLOT_CACHE: FrameSlotCache = const {
        FrameSlotCache {
            current_frame: AtomicUsize::new(0),
            #[cfg(all(unix, feature = "threading"))]
            top_frame: Cell::new(core::ptr::null()),
            #[cfg(feature = "threading")]
            top_iframe: Cell::new(core::ptr::null()),
        }
    };

    /// Cached pointer to this thread's `ThreadSlot::stop_requested`, for the
    /// safepoint the dispatch loop takes once per instruction. Reading it
    /// through `CURRENT_THREAD_SLOT` costs a `RefCell` borrow — two stores to
    /// thread-local memory — where this costs one relaxed load. The slot's Arc
    /// keeps the pointee alive, as with the frame pointers above.
    #[cfg(feature = "threading")]
    static CURRENT_STOP_REQUESTED: Cell<*const core::sync::atomic::AtomicBool> =
        const { Cell::new(core::ptr::null()) };

}

fn callbacks_permitted() -> bool {
    if NATIVE_SWEEP.try_with(Cell::get).unwrap_or(true) {
        return false;
    }
    NATIVE_TEARDOWN
        .try_with(|state| state.borrow().is_none())
        .unwrap_or(false)
}

pub(crate) fn native_sweep<R>(f: impl FnOnce() -> R) -> R {
    let previous = NATIVE_SWEEP.with(|sweep| sweep.replace(true));
    scopeguard::defer! {
        NATIVE_SWEEP.with(|sweep| sweep.set(previous));
        if !previous {
            drain_deferred_vms();
        }
    }
    f()
}

pub(crate) fn native_teardown<R>(
    state: &crate::common::rc::PyRc<PyGlobalState>,
    f: impl FnOnce() -> R,
) -> R {
    NATIVE_TEARDOWN.with(|current| {
        assert!(current.borrow().is_none(), "nested native teardown");
        *current.borrow_mut() = Some(state.clone());
    });
    scopeguard::defer! {
        let state = NATIVE_TEARDOWN.with(|current| current.borrow_mut().take());
        drop(state);
    }
    f()
}

pub(super) fn destroy_vm(vm: Box<VirtualMachine>) {
    if !callbacks_permitted() {
        DEFERRED_VMS.with(|pending| pending.borrow_mut().push(vm));
        return;
    }
    scopeguard::defer! { drain_deferred_vms(); }
    let state = vm.state.clone();
    let Some((vm, lease, owns_vm)) = state.owner_leases.begin_teardown(vm) else {
        return;
    };
    let teardown = scopeguard::guard(&state, |state| {
        lease.finish_teardown();
        state.teardowns.fetch_sub(1, Ordering::Release);
    });
    #[cfg(feature = "threading")]
    let has_outer_owner = VM_STACK.with(|stack| {
        stack.borrow().iter().any(|vm| {
            // SAFETY: stack entries remain borrowed by their enclosing entry.
            unsafe { vm.as_ref() }.state.interpreter_id == state.interpreter_id
        })
    });
    let closed = state.closed.load(Ordering::Acquire) || !owns_vm;
    let mut entered = VmBootstrapGuard::enter_with_owner(&vm, closed, lease.enter());
    let released = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
        if !closed {
            let mut cleanup = super::owned::Cleanup::default();
            cleanup.run(|| vm.retire_execution_roots());
            #[cfg(feature = "threading")]
            if !has_outer_owner {
                cleanup.run(|| retire_thread_slot(&vm));
            }
            cleanup.finish();
        }
    }));
    let last = if owns_vm {
        lease.release_owner();
        state.owners.fetch_sub(1, Ordering::AcqRel) == 1
    } else {
        false
    };
    let _finalization = (last && !closed)
        .then(|| state.owner_leases.enter_finalization(&state))
        .flatten();
    let finalized = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
        let mut cleanup = super::owned::Cleanup::default();
        if last && !closed {
            state.admission_closed.store(true, Ordering::Release);
            #[cfg(feature = "threading")]
            state
                .finalizing_thread_ident
                .store(crate::stdlib::_thread::get_ident());
            state.finalizing.store(true, Ordering::Release);
            cleanup.run(|| state.roots.close());
            if vm.initialized.get() {
                cleanup.run(|| vm.finalize_modules());
            }
            cleanup.run(|| {
                crate::stdlib::_interpchannels::clear_interpreter(state.interpreter_id);
            });
            cleanup.run(|| {
                crate::stdlib::_interpqueues::clear_interpreter(state.interpreter_id);
            });
        }
        if !closed {
            // SAFETY: OwnedVm destruction runs after this VM's execution scopes
            // have ended. The cleanup entry has not borrowed its cached roots.
            cleanup.run(|| unsafe { vm.retire_cached_roots() });
            if last {
                #[cfg(feature = "threading")]
                {
                    cleanup.run(|| retire_interpreter_thread_roots(&vm));
                }
                cleanup.run(|| state.retire_python_roots(&vm.ctx));
                cleanup.run(|| {
                    state.gc.collect_force(2);
                });
            }
        }
        cleanup.finish();
    }));
    let entry = &mut entered;
    let state_ref = &state;
    let lease_ref = &lease;
    let terminal = native_teardown(&state, move || {
        // No TLS pointer may refer to a VM while its fields are being dropped.
        // Pop before any native destructor can panic, including cache cleanup.
        lease_ref.retire_vm();
        entry.pop_vm();
        let result = std::panic::catch_unwind(core::panic::AssertUnwindSafe(move || {
            let mut cleanup = super::owned::Cleanup::default();
            if last {
                state_ref.closed.store(true, Ordering::Release);
                cleanup.run(|| state_ref.close_python_roots());
                #[cfg(feature = "threading")]
                cleanup.run(|| retire_interpreter_thread_roots(&vm));
            }
            #[cfg(feature = "threading")]
            if !has_outer_owner {
                cleanup.run(|| retire_thread_slot(&vm));
            }
            cleanup.run(|| drop(vm));
            if last {
                cleanup.run(|| state_ref.gc.unfreeze());
                cleanup.run(|| {
                    state_ref.gc.collect_force(2);
                });
            }
            cleanup.finish();
        }));
        // Unwind has dropped VM storage under the callback mask. Keep the slot
        // attached until then, and unregister it before restoring an outer VM.
        #[cfg(feature = "threading")]
        if !has_outer_owner {
            unregister_current_thread_frames(state_ref);
        }
        result
    });
    drop(entered);
    drop(teardown);
    if let Err(error) = released.and(finalized).and(terminal) {
        std::panic::resume_unwind(error);
    }
}

fn drain_deferred_vms() {
    if !callbacks_permitted() || DRAINING_VMS.with(|draining| draining.replace(true)) {
        return;
    }
    scopeguard::defer! { DRAINING_VMS.with(|draining| draining.set(false)); }
    let mut cleanup = super::owned::Cleanup::default();
    while let Some(vm) = DEFERRED_VMS.with(|pending| pending.borrow_mut().pop()) {
        cleanup.run(|| destroy_vm(vm));
    }
    cleanup.finish();
}

#[must_use]
pub fn current_vm_is_set() -> bool {
    VM_STACK.with(|vms| !vms.borrow().is_empty())
}

pub(crate) fn is_current_attached(vm: &VirtualMachine) -> bool {
    top_slot_is_attached()
        && try_with_current_vm(|current| core::ptr::eq(current, vm)).unwrap_or(false)
}

pub(crate) fn try_with_attached_vm<R>(f: impl FnOnce(&VirtualMachine) -> R) -> Option<R> {
    if !callbacks_permitted() {
        return None;
    }
    let vm = VM_STACK
        .try_with(|stack| stack.try_borrow().ok()?.last().copied())
        .ok()??;
    #[cfg(feature = "threading")]
    {
        let attached = CURRENT_THREAD_SLOT
            .try_with(|slot| {
                slot.try_borrow()
                    .ok()
                    .and_then(|slot| {
                        slot.as_ref().map(|slot| {
                            slot.state.load(Ordering::Acquire) == ThreadState::Attached as i32
                        })
                    })
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !attached {
            return None;
        }
    }
    // SAFETY: the active VM stack retains this reference until the entry exits.
    Some(f(unsafe { vm.as_ref() }))
}

pub(crate) fn with_current_vm<R>(f: impl FnOnce(&VirtualMachine) -> R) -> R {
    assert!(callbacks_permitted(), "Python entry during native teardown");
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
    assert!(callbacks_permitted(), "Python entry during native teardown");
    let _guard = VmBootstrapGuard::new(vm);
    f()
}

/// Pointer to the GC state of the interpreter running on this thread.
///
/// The pointee belongs to the `PyGlobalState` of the VM on top of `VM_STACK`,
/// which is borrowed for the whole `set_current_vm` scope — so the pointer stays
/// valid as long as the caller remains inside that scope.
pub(crate) fn current_gc_state() -> Option<NonNull<crate::gc_state::GcInterpreterState>> {
    if NATIVE_SWEEP.try_with(Cell::get).unwrap_or(true) {
        return None;
    }
    if let Some(gc) = NATIVE_TEARDOWN
        .try_with(|state| {
            state
                .borrow()
                .as_ref()
                .map(|state| NonNull::from(&state.gc))
        })
        .ok()
        .flatten()
    {
        return Some(gc);
    }
    // Reached from every tracked allocation, including ones a thread-local
    // destructor makes while the VM stack is being torn down, so neither a
    // destroyed key nor an outstanding borrow may panic here.
    VM_STACK
        .try_with(|vms| {
            let vm = vms.try_borrow().ok()?.last().copied()?;
            // SAFETY: entries in VM_STACK either borrow a VM for the dynamic
            // scope of a set_current_vm()/enter_vm() call or point at GILSTATE_VM.
            Some(NonNull::from(&unsafe { vm.as_ref() }.state.gc))
        })
        .ok()
        .flatten()
}

pub(crate) fn try_with_current_vm<R>(f: impl FnOnce(&VirtualMachine) -> R) -> Option<R> {
    if !callbacks_permitted() {
        return None;
    }
    VM_STACK.with(|vms| {
        let vm = vms.borrow().last().copied()?;
        // SAFETY: entries in VM_STACK either borrow a VM for the dynamic
        // scope of a set_current_vm()/enter_vm() call or point at GILSTATE_VM.
        Some(f(unsafe { vm.as_ref() }))
    })
}

pub fn enter_vm<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    // Attach/detach is handled by `set_current_vm`, which pairs it with the
    // VM_STACK push so that switching interpreters mid-stack stays consistent.
    set_current_vm(vm, || {
        vm.state.roots.drain_pending();
        scopeguard::defer! { vm.state.roots.drain_pending(); }
        f()
    })
}

/// RAII counterpart to `enter_vm` for bootstrap and owner teardown. The caller
/// keeps the VM at a stable address and may only publish shared access until
/// the guard is popped; in particular, publication forbids borrowing it mutably.
///
/// Without this, code that runs Python bytecode before any `enter_vm` scope
/// exists would leave the thread not ATTACHED, making lock-free type cache
/// reads unsound.
#[must_use]
pub(crate) struct VmBootstrapGuard {
    #[cfg(feature = "threading")]
    switched: bool,
    pushed: bool,
    saved_frame: Option<*const InterpreterFrame>,
    _owner: super::owner_lease::OwnerEntry,
}

impl VmBootstrapGuard {
    pub(crate) fn new(vm: &VirtualMachine) -> Self {
        Self::enter(vm, false)
    }

    fn enter(vm: &VirtualMachine, cleanup: bool) -> Self {
        Self::enter_with_owner(vm, cleanup, vm.owner_lease().enter())
    }

    fn enter_with_owner(
        vm: &VirtualMachine,
        cleanup: bool,
        owner: super::owner_lease::OwnerEntry,
    ) -> Self {
        assert!(
            callbacks_permitted(),
            "Python bootstrap during native teardown"
        );
        let changed_vm = VM_STACK.with(|vms| {
            vms.borrow()
                .last()
                .is_none_or(|p| p.as_ptr().cast_const() != core::ptr::from_ref(vm))
        });
        let saved_frame = changed_vm.then(get_current_frame);
        #[cfg(feature = "threading")]
        let switched = {
            if cleanup {
                let slot = ensure_thread_slot(vm);
                // A cancelled/idle native worker can be dropped after explicit
                // shutdown. Cleanup is not a new Python entry and must not hang.
                let _ = slot.state.compare_exchange(
                    ThreadState::ShuttingDown as i32,
                    ThreadState::Detached as i32,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
            begin_interpreter_section(vm)
        };
        #[cfg(not(feature = "threading"))]
        let _ = cleanup;
        if changed_vm {
            let _ = set_current_frame(core::ptr::null());
        }

        VM_STACK.with(|vms| vms.borrow_mut().push(vm.into()));

        Self {
            #[cfg(feature = "threading")]
            switched,
            pushed: true,
            saved_frame,
            _owner: owner,
        }
    }

    fn pop_vm(&mut self) {
        if self.pushed {
            VM_STACK.with(|vms| {
                vms.borrow_mut().pop();
            });
            self.pushed = false;
        }
    }
}

impl Drop for VmBootstrapGuard {
    fn drop(&mut self) {
        self.pop_vm();
        #[cfg(feature = "threading")]
        end_interpreter_section(self.switched);
        if let Some(frame) = self.saved_frame {
            let _ = set_current_frame(frame);
        }
    }
}

#[cfg(feature = "threading")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CurrentVmAttachState {
    AlreadyAttached,
    Attached,
}

/// State preserved while the current native thread is detached from its VM.
#[cfg(feature = "threading")]
pub struct SavedThreadState {
    vm_stack: Vec<NonNull<VirtualMachine>>,
    gilstate_vm: Option<Box<ThreadedVirtualMachine>>,
    gilstate_entry: Option<super::owner_lease::OwnerEntry>,
    frame: *const InterpreterFrame,
}

/// Detach the current native thread and preserve its VM context for restoration.
#[cfg(feature = "threading")]
#[must_use = "the saved thread state must be restored"]
pub(crate) fn save_current_thread() -> SavedThreadState {
    let vm_stack = VM_STACK.with(|vms| core::mem::take(&mut *vms.borrow_mut()));
    assert!(
        !vm_stack.is_empty(),
        "save_current_thread() called without an attached VM"
    );
    let gilstate_vm = GILSTATE_VM.with(|gilstate_vm| gilstate_vm.borrow_mut().take());
    let gilstate_entry = GILSTATE_ENTRY.with(|entry| entry.borrow_mut().take());
    detach_thread();
    // Keep the old interpreter's published frame intact for other threads,
    // but do not let a subsequently attached interpreter inherit this chain.
    let frame = FRAME_SLOT_CACHE.with(|cache| cache.current_frame.swap(0, Ordering::Relaxed))
        as *const InterpreterFrame;
    SavedThreadState {
        vm_stack,
        gilstate_vm,
        gilstate_entry,
        frame,
    }
}

/// Restore a VM context previously returned by [`save_current_thread`].
#[cfg(feature = "threading")]
pub(crate) fn restore_current_thread(state: SavedThreadState) {
    assert!(
        callbacks_permitted(),
        "thread restoration during native teardown"
    );
    assert!(
        !current_vm_is_set(),
        "restore_current_thread() called with an attached VM"
    );
    let SavedThreadState {
        vm_stack,
        gilstate_vm,
        gilstate_entry,
        frame,
    } = state;
    let vm = vm_stack
        .last()
        .copied()
        .expect("saved thread state has no VM");

    GILSTATE_VM.with(|current| {
        let mut current = current.borrow_mut();
        assert!(
            current.is_none(),
            "restore_current_thread() called with a GILState VM"
        );
        *current = gilstate_vm;
    });
    GILSTATE_ENTRY.with(|current| {
        let mut current = current.borrow_mut();
        assert!(current.is_none());
        *current = gilstate_entry;
    });

    // SAFETY: borrowed VMs remain alive for the dynamic save/restore scope,
    // while an owned GILState VM was restored above before this dereference.
    let vm = unsafe { vm.as_ref() };
    // Point CURRENT_THREAD_SLOT at the restored interpreter before attach.
    // After subinterpreter bootstrap, CURRENT may still refer to the temporary
    // subinterpreter slot (DETACHED); attaching that would leave the parent
    // slot detached and later confuse outermost detach.
    init_thread_slot_if_needed(vm);
    attach_thread(vm);
    VM_STACK.with(|vms| *vms.borrow_mut() = vm_stack);
    let _ = set_current_frame(frame);
}

/// Attach the current native thread to a RustPython VM until
/// `release_current_thread()` is called.
///
/// # Safety
/// Pair a newly attached state with exactly one release on this native thread.
/// No Python reference or scoped VM token may outlive that attachment.
#[cfg(feature = "threading")]
pub unsafe fn attach_current_thread(
    make_vm: impl FnOnce() -> ThreadedVirtualMachine,
) -> CurrentVmAttachState {
    assert!(callbacks_permitted(), "native attachment during teardown");
    if current_vm_is_set() {
        return CurrentVmAttachState::AlreadyAttached;
    }

    GILSTATE_VM.with(|gilstate_vm| {
        let mut gilstate_vm = gilstate_vm.borrow_mut();
        let threaded_vm = gilstate_vm.get_or_insert_with(|| Box::new(make_vm()));
        let vm: &VirtualMachine = &threaded_vm.vm;
        let entry = vm.owner_lease().enter();

        vm.c_stack_soft_limit
            .set(VirtualMachine::calculate_c_stack_soft_limit());

        init_thread_slot_if_needed(vm);

        attach_thread(vm);

        VM_STACK.with(|vms| {
            debug_assert!(vms.borrow().is_empty());
            vms.borrow_mut().push(vm.into());
        });
        GILSTATE_ENTRY.with(|current| *current.borrow_mut() = Some(entry));
    });

    CurrentVmAttachState::Attached
}

/// Release a native C-API attachment.
///
/// # Safety
/// `state` must be the unmatched result of `attach_current_thread` on this
/// thread. All references and callbacks using that attachment must have ended.
#[cfg(feature = "threading")]
pub unsafe fn release_current_thread(state: CurrentVmAttachState) {
    if state == CurrentVmAttachState::AlreadyAttached {
        return;
    }

    VM_STACK.with(|vms| {
        vms.borrow_mut()
            .pop()
            .expect("release_current_thread() called without an attached VM");
    });

    let gilstate_vm = GILSTATE_VM.with(|gilstate_vm| gilstate_vm.borrow_mut().take());
    drop(gilstate_vm);

    detach_thread();
    GILSTATE_ENTRY.with(|entry| entry.borrow_mut().take());
}

/// Ensure this OS thread has a [`ThreadSlot`] registered with `vm`'s interpreter
/// and make it the current slot.
///
/// Called automatically by `enter_vm()` / `VmBootstrapGuard` whenever a VM
/// becomes current. Switching between interpreters on the same OS thread swaps
/// `CURRENT_THREAD_SLOT` to that interpreter's slot (creating one if needed).
#[cfg(feature = "threading")]
fn init_thread_slot_if_needed(vm: &VirtualMachine) {
    let slot = ensure_thread_slot(vm);
    set_current_thread_slot(slot);
}

/// Look up (creating if needed) this thread's [`ThreadSlot`] for `vm`'s
/// interpreter, without making it the current slot.
#[cfg(feature = "threading")]
fn ensure_thread_slot(vm: &VirtualMachine) -> CurrentFrameSlot {
    let interp_id = vm.state.interpreter_id;
    INTERP_THREAD_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        if let Some(existing) = slots.get(&interp_id) {
            return existing.clone();
        }

        let thread_id = crate::stdlib::_thread::get_ident();
        let mut registry = vm.state.thread_frames.lock();
        let new_slot = Arc::new(ThreadSlot {
            closed: core::sync::atomic::AtomicBool::new(false),
            #[cfg(unix)]
            top_frame: AtomicPtr::new(core::ptr::null_mut()),
            top_iframe: AtomicUsize::new(0),
            #[cfg(not(unix))]
            frames: parking_lot::Mutex::new(Vec::new()),
            exception: crate::PyAtomicRef::from(None::<PyBaseExceptionRef>),
            trace_func: PyMutex::new(
                vm.state
                    .global_trace_func
                    .lock()
                    .clone()
                    .unwrap_or_else(|| vm.ctx.none()),
            ),
            profile_func: PyMutex::new(
                vm.state
                    .global_profile_func
                    .lock()
                    .clone()
                    .unwrap_or_else(|| vm.ctx.none()),
            ),
            state: core::sync::atomic::AtomicI32::new(
                if vm.state.stop_the_world.requested.load(Ordering::Acquire) {
                    // Match init_threadstate(): new thread-state starts
                    // suspended while stop-the-world is active.
                    ThreadState::Suspended as i32
                } else {
                    ThreadState::Detached as i32
                },
            ),
            stop_requested: core::sync::atomic::AtomicBool::new(false),
            thread: std::thread::current(),
            qsbr: vm.state.gc.qsbr().register(),
            shared_qsbr: crate::object::qsbr::shared().register(),
        });
        registry.insert(thread_id, new_slot.clone());
        drop(registry);
        slots.insert(interp_id, new_slot.clone());
        new_slot
    })
}

/// The current thread's `ThreadSlot` for the entered interpreter, if any.
#[cfg(feature = "threading")]
#[must_use]
pub(crate) fn current_thread_slot() -> Option<CurrentFrameSlot> {
    CURRENT_THREAD_SLOT.with(|slot| slot.borrow().clone())
}

/// Make `slot` the current thread slot (and the cached top-frame pointer).
#[cfg(feature = "threading")]
fn set_current_thread_slot(slot: CurrentFrameSlot) {
    FRAME_SLOT_CACHE.with(|cache| {
        #[cfg(unix)]
        cache.top_frame.set(&slot.top_frame);
        cache.top_iframe.set(&slot.top_iframe);
    });
    CURRENT_STOP_REQUESTED.with(|c| c.set(&slot.stop_requested));
    CURRENT_THREAD_SLOT.with(|current| {
        *current.borrow_mut() = Some(slot);
    });
}

/// Whether the current thread slot is ATTACHED.
#[cfg(feature = "threading")]
fn current_slot_is_attached() -> bool {
    CURRENT_THREAD_SLOT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|s| s.state.load(Ordering::Acquire) == ThreadState::Attached as i32)
    })
}

/// Attach this thread to `vm`'s interpreter for the duration of a section,
/// detaching whichever interpreter it was attached to (≈ `_PyThreadState_Swap`).
///
/// A thread must never be ATTACHED to two interpreters at once: stop-the-world
/// treats an ATTACHED slot as "running this interpreter's bytecode" and a
/// DETACHED slot as parkable without cooperation, so running interpreter B's
/// code while B's slot is DETACHED would let a collector conclude B is stopped
/// while this thread keeps mutating the (process-global) object graph.
///
/// Returns whether the attachment changed, i.e. whether the matching
/// [`end_interpreter_section`] must undo it.
#[cfg(feature = "threading")]
fn begin_interpreter_section(vm: &VirtualMachine) -> bool {
    let target = ensure_thread_slot(vm);
    let already_current = CURRENT_THREAD_SLOT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|s| Arc::ptr_eq(s, &target))
    });
    if already_current && current_slot_is_attached() {
        // Nested section in the same interpreter: already attached.
        return false;
    }
    if !already_current && current_slot_is_attached() {
        detach_thread();
    }
    set_current_thread_slot(target);
    attach_thread(vm);
    true
}

/// Undo [`begin_interpreter_section`]: detach this interpreter and re-attach the
/// enclosing one, if any. Call after the VM has been popped from `VM_STACK`.
#[cfg(feature = "threading")]
fn end_interpreter_section(switched: bool) {
    if !switched {
        return;
    }
    if current_slot_is_attached() {
        detach_thread();
    }
    // The enclosing section, if any, is the VM now on top of the stack.
    if let Some(vm_ptr) = VM_STACK.with(|vms| vms.borrow().last().copied()) {
        // SAFETY: entries on VM_STACK are valid for their enter/set_current_vm scope.
        let vm = unsafe { vm_ptr.as_ref() };
        set_current_thread_slot(ensure_thread_slot(vm));
        attach_thread(vm);
    }
}

/// Transition DETACHED → ATTACHED. Blocks if the thread was SUSPENDED by
/// a stop-the-world request (like `_PyThreadState_Attach` + `tstate_wait_attach`).
#[cfg(feature = "threading")]
fn wait_while_suspended(slot: &ThreadSlot) -> u64 {
    let mut wait_yields = 0u64;
    while slot.state.load(Ordering::Acquire) == ThreadState::Suspended as i32 {
        wait_yields = wait_yields.saturating_add(1);
        std::thread::park();
    }
    wait_yields
}

/// `PyThread_hang_thread`: park this OS thread forever.
#[cfg(feature = "threading")]
fn hang_thread() -> ! {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(slot) = slot.borrow().as_ref() {
            slot.qsbr.offline();
            slot.shared_qsbr.offline();
        }
    });
    loop {
        std::thread::park();
    }
}

/// `_PyThreadState_HangThread`: this thread may no longer run Python.
///
/// Mark the slot shutting-down (so later stop-the-world requests do not wait
/// for it) and never return. The matching `_ThreadHandle` stays not-done, so
/// `Thread.is_alive()` remains true for a daemon forced off during finalize.
#[cfg(feature = "threading")]
pub fn hang_current_thread(state: &PyGlobalState) -> ! {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            let prev = s
                .state
                .swap(ThreadState::ShuttingDown as i32, Ordering::AcqRel);
            if prev == ThreadState::Attached as i32 {
                s.qsbr.offline();
                s.shared_qsbr.offline();
            }
            s.stop_requested.store(false, Ordering::Release);
        }
    });
    if state.stop_the_world.requested.load(Ordering::Acquire) {
        state.stop_the_world.notify_thread_gone();
    }
    hang_thread();
}

/// `_PyThreadState_SetShuttingDown` on every non-current thread.
///
/// Call while the world is stopped. Wake parked threads so they observe
/// `SHUTTING_DOWN` and hang on the next attach, instead of resuming Python.
#[cfg(feature = "threading")]
pub fn set_other_threads_shutting_down(state: &PyGlobalState) {
    let current = crate::stdlib::_thread::get_ident();
    let registry = state.thread_frames.lock();

    #[expect(
        clippy::iter_over_hash_type,
        reason = "Iteration order doesn't matter here"
    )]
    for (&id, slot) in registry.iter() {
        if id == current {
            continue;
        }
        slot.stop_requested.store(false, Ordering::Release);
        slot.state
            .store(ThreadState::ShuttingDown as i32, Ordering::Release);
        slot.qsbr.offline();
        slot.shared_qsbr.offline();
        slot.thread.unpark();
    }
}

#[cfg(feature = "threading")]
fn attach_thread(vm: &VirtualMachine) {
    attach_state(&vm.state);
}

#[cfg(feature = "threading")]
fn attach_state(interpreter: &PyGlobalState) {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            super::stw_trace(format_args!("attach begin"));
            loop {
                match s.state.compare_exchange(
                    ThreadState::Detached as i32,
                    ThreadState::Attached as i32,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        s.qsbr.online();
                        s.shared_qsbr.online();
                        super::stw_trace(format_args!("attach DETACHED->ATTACHED"));
                        break;
                    }
                    Err(state) => match ThreadState::from_i32(state) {
                        Some(ThreadState::Suspended) => {
                            // Parked by stop-the-world — wait until released to DETACHED
                            super::stw_trace(format_args!("attach wait-suspended"));
                            let wait_yields = wait_while_suspended(s);
                            interpreter
                                .stop_the_world
                                .add_attach_wait_yields(wait_yields);
                            // Retry CAS
                        }
                        Some(ThreadState::ShuttingDown) => {
                            super::stw_trace(format_args!("attach hang shutting-down"));
                            hang_thread();
                        }
                        _ => {
                            debug_assert!(false, "unexpected thread state in attach: {state}");
                            break;
                        }
                    },
                }
            }
        }
    });
    // A stop-the-world may have been requested while this thread was detached.
    // Honoring it here (rather than only at the next bytecode safepoint) keeps
    // a thread doing rapid allow_threads calls from re-attaching and running
    // past the requester forever, which would stall stop-the-world. Done
    // outside the CURRENT_THREAD_SLOT borrow above because suspend re-borrows
    // it. Safe against a concurrent start_the_world: suspend_if_needed decides
    // whether to park under the registry lock, so it never parks after the
    // request has been withdrawn.
    suspend_if_needed(interpreter);
}

/// Transition ATTACHED → DETACHED (like `_PyThreadState_Detach`).
#[cfg(feature = "threading")]
fn detach_thread() {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            match s.state.compare_exchange(
                ThreadState::Attached as i32,
                ThreadState::Detached as i32,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    s.qsbr.offline();
                    s.shared_qsbr.offline();
                }
                Err(state) => {
                    debug_assert!(
                        matches!(ThreadState::from_i32(state), Some(ThreadState::Detached)),
                        "unexpected thread state in detach: {state}"
                    );
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
#[cfg(feature = "threading")]
pub fn allow_threads<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    // Preserve save/restore semantics:
    // only detach if this call observed ATTACHED at entry, and always restore
    // on unwind.
    let should_transition = CURRENT_THREAD_SLOT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|s| s.state.load(Ordering::Acquire) == ThreadState::Attached as i32)
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

/// No-op on non-threading builds.
#[cfg(not(feature = "threading"))]
pub fn allow_threads<R>(_vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    f()
}

/// Run `f` with this thread attached, then return it to where it was.
///
/// The inverse of [`allow_threads`], for a callback that has to run Python from
/// inside a call the thread detached for — a handshake callback reaching a
/// Python `sni_callback`, say. Running that detached would execute Python on a
/// thread a stop-the-world requester counts as parked. `PyGILState_Ensure` and
/// `PyGILState_Release` bracket such a callback for the same reason.
///
/// A thread already attached, or one with no interpreter to attach to, just
/// runs `f`. A thread a stop-the-world has already moved to SUSPENDED parks
/// here until the world starts again, because [`attach_thread`] treats that
/// state as the wait it is; that is the point of routing through it rather than
/// testing for DETACHED alone.
#[cfg(feature = "threading")]
pub fn attach_for_callback<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    assert!(
        callbacks_permitted(),
        "Python callback during native teardown"
    );
    let should_transition = CURRENT_THREAD_SLOT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|s| s.state.load(Ordering::Acquire) != ThreadState::Attached as i32)
    });
    if !should_transition {
        return f();
    }

    attach_thread(vm);
    // Detach again even if `f` unwinds, so the `allow_threads` this is nested
    // inside still finds the state it left behind.
    let redetach_guard = scopeguard::guard((), |()| detach_thread());
    let result = f();
    drop(redetach_guard);
    result
}

/// No-op on non-threading builds.
#[cfg(not(feature = "threading"))]
pub fn attach_for_callback<R>(_vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    assert!(
        callbacks_permitted(),
        "Python callback during native teardown"
    );
    f()
}

/// Wait for a lock the way a blocking call waits: detached, so a
/// stop-the-world requester never has to wait for this thread to reach a
/// safepoint it cannot reach while blocked.
///
/// Threads with no interpreter to leave — a native thread, or one whose
/// locals are already being destroyed — simply block.
///
/// Detaching cannot park the one thread that can start the world again:
/// [`park_detached_threads`](super::StopTheWorldState) skips the requester's
/// slot outright, by thread id, and [`suspend_if_needed`] keys off a stop bit
/// never set for it. That exemption is wider than the one `_PyEval_StopTheWorld`
/// gives, where only an ATTACHED requester is skipped and a DETACHED one is
/// suspended like any other thread — so this rests on a local invariant rather
/// than on the reference behavior.
#[cfg(feature = "threading")]
fn wait_detached_from_interpreter(wait: &dyn Fn()) {
    let terminal = NATIVE_TEARDOWN
        .try_with(|state| state.borrow().clone())
        .ok()
        .flatten();
    if let Some(state) = terminal {
        if current_slot_is_attached() {
            detach_thread();
            scopeguard::defer! { attach_state(&state); }
            wait();
        } else {
            wait();
        }
        return;
    }
    // Read the VM out before waiting: attaching afterwards reaches for the
    // same thread locals, which must not still be borrowed here.
    let current = VM_STACK
        .try_with(|vms| vms.try_borrow().ok()?.last().copied())
        .ok()
        .flatten();
    match current {
        // SAFETY: entries in VM_STACK either borrow a VM for the dynamic
        // scope of a set_current_vm()/enter_vm() call or point at GILSTATE_VM.
        Some(vm) => allow_threads(unsafe { vm.as_ref() }, wait),
        None => wait(),
    }
}

/// Teach the lock types how to detach this thread. Idempotent, so every
/// interpreter can call it while initializing.
#[cfg(feature = "threading")]
pub(crate) fn install_blocking_wait_hook() {
    rustpython_common::lock::set_blocking_wait_hook(wait_detached_from_interpreter);
}

/// Called from check_signals when stop-the-world is requested.
/// Transitions ATTACHED → SUSPENDED and waits until released
/// (like `_PyThreadState_Suspend` + `_PyThreadState_Attach`).
#[cfg(feature = "threading")]
pub fn suspend_if_needed(state: &PyGlobalState) {
    let should_suspend = CURRENT_THREAD_SLOT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|s| s.stop_requested.load(Ordering::Relaxed))
    });
    if should_suspend {
        do_suspend(state);
    }
}

#[cfg(feature = "threading")]
#[cold]
fn do_suspend(state: &PyGlobalState) {
    let stw = &state.stop_the_world;
    CURRENT_THREAD_SLOT.with(|slot| {
        let borrowed = slot.borrow();
        let Some(s) = borrowed.as_ref() else {
            return;
        };

        // Decide whether to park while holding the thread registry. Both edges
        // of `requested` are written under that lock: `init_thread_countdown`
        // sets it, and `start_the_world` clears it and then releases every
        // SUSPENDED thread without letting go. Publishing SUSPENDED here is
        // therefore either seen by that release pass or never reached, which
        // leaves the requester the only writer that takes a thread out of
        // SUSPENDED. A completion check that observed this thread parked cannot
        // then be invalidated by the thread resuming on its own.
        let park = {
            let _registry = state.thread_frames.lock();
            if stw.requested.load(Ordering::Acquire) {
                Some(s.state.compare_exchange(
                    ThreadState::Attached as i32,
                    ThreadState::Suspended as i32,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ))
            } else {
                // The stop already ended; this thread's request bit is stale.
                s.stop_requested.store(false, Ordering::Release);
                None
            }
        };

        match park {
            None => {
                super::stw_trace(format_args!("suspend skip not-requested"));
                return;
            }
            Some(Ok(_)) => {
                // Consumed this thread's stop request bit.
                s.stop_requested.store(false, Ordering::Release);
                s.qsbr.offline();
                s.shared_qsbr.offline();
            }
            Some(Err(state)) => match ThreadState::from_i32(state) {
                Some(ThreadState::Detached) => {
                    // Leaving VM; caller will re-check on next entry.
                    super::stw_trace(format_args!("suspend skip DETACHED"));
                    return;
                }
                Some(ThreadState::Suspended) => {
                    // Already parked by another path.
                    s.stop_requested.store(false, Ordering::Release);
                    super::stw_trace(format_args!("suspend skip already-suspended"));
                    return;
                }
                Some(ThreadState::ShuttingDown) => {
                    s.stop_requested.store(false, Ordering::Release);
                    super::stw_trace(format_args!("suspend hang shutting-down"));
                    hang_thread();
                }
                _ => {
                    debug_assert!(false, "unexpected thread state in suspend: {state}");
                    return;
                }
            },
        }
        super::stw_trace(format_args!("suspend ATTACHED->SUSPENDED"));

        // Notify the stop-the-world requester that we've parked. The registry
        // is released first: the requester's wait loop takes the notify mutex
        // and then the registry, so taking them the other way round here would
        // invert the order.
        stw.notify_suspended();
        super::stw_trace(format_args!("suspend notified-requester"));

        // Wait until start_the_world sets us back to DETACHED
        let wait_yields = wait_while_suspended(s);
        stw.add_suspend_wait_yields(wait_yields);

        // Re-attach (DETACHED → ATTACHED), tstate_wait_attach CAS loop.
        loop {
            match s.state.compare_exchange(
                ThreadState::Detached as i32,
                ThreadState::Attached as i32,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(state) => match ThreadState::from_i32(state) {
                    Some(ThreadState::Suspended) => {
                        let extra_wait = wait_while_suspended(s);
                        stw.add_suspend_wait_yields(extra_wait);
                    }
                    Some(ThreadState::Attached) => break,
                    Some(ThreadState::ShuttingDown) => {
                        super::stw_trace(format_args!("suspend resume hang shutting-down"));
                        hang_thread();
                    }
                    _ => {
                        debug_assert!(false, "unexpected post-suspend state: {state}");
                        break;
                    }
                },
            }
        }
        s.qsbr.online();
        s.shared_qsbr.online();
        s.stop_requested.store(false, Ordering::Release);
        super::stw_trace(format_args!("suspend resume -> ATTACHED"));
    });
}

#[cfg(feature = "threading")]
#[inline]
#[must_use]
pub fn stop_requested_for_current_thread() -> bool {
    CURRENT_STOP_REQUESTED.with(|cached| {
        let flag = cached.get();
        // SAFETY: the pointer is non-null only while `CURRENT_THREAD_SLOT`
        // holds the `Arc<ThreadSlot>` that owns the flag; both are cleared
        // together in `cleanup_current_thread_frames`.
        !flag.is_null() && unsafe { &*flag }.load(Ordering::Relaxed)
    })
}

#[cfg(all(test, feature = "threading"))]
pub(crate) fn set_stop_requested_for_current_thread(value: bool) -> bool {
    CURRENT_STOP_REQUESTED.with(|cached| {
        let flag = cached.get();
        if flag.is_null() {
            return false;
        }
        // SAFETY: same lifetime as `stop_requested_for_current_thread`.
        unsafe { &*flag }.store(value, Ordering::Release);
        true
    })
}

/// Whether the QSBR subsystem asked this thread to pass a checkpoint.
/// A missed or racing read of this flag is harmless: the pending
/// retirement is still processed at the next checkpoint or by the GC
/// backstop.
#[cfg(feature = "threading")]
pub(crate) fn qsbr_break_requested() -> bool {
    CURRENT_THREAD_SLOT.with(|slot| {
        slot.borrow().as_ref().is_some_and(|s| {
            s.qsbr.requested.load(Ordering::Relaxed)
                || s.shared_qsbr.requested.load(Ordering::Relaxed)
        })
    })
}

/// Pass a QSBR checkpoint: the calling thread holds no borrowed cache
/// pointers here (instruction boundary), so mark it quiescent and try to
/// free retired allocations.
#[cfg(feature = "threading")]
pub(crate) fn qsbr_checkpoint() {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            s.qsbr.checkpoint();
            s.shared_qsbr.checkpoint();
        }
    });
}

/// Debug check: lock-free type-cache reads are only sound on threads that
/// are registered with QSBR and currently ATTACHED.
#[cfg(all(feature = "threading", debug_assertions))]
pub(crate) fn debug_assert_current_thread_attached() {
    CURRENT_THREAD_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            debug_assert_eq!(
                s.state.load(Ordering::Relaxed),
                ThreadState::Attached as i32,
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
pub(crate) fn push_thread_frame(fp: FramePtr) {
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
pub(crate) fn pop_thread_frame() {
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

/// Set the current thread's top InterpreterFrame pointer.
/// Returns the previous pointer so it can be restored on pop.
#[must_use]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub(crate) fn set_current_frame(frame: *const InterpreterFrame) -> *const InterpreterFrame {
    FRAME_SLOT_CACHE.with(|cache| {
        // Publish the top frame for cross-thread readers (faulthandler,
        // sys._current_frames).
        #[cfg(feature = "threading")]
        {
            let slot = cache.top_iframe.get();
            if !slot.is_null() {
                unsafe { &*slot }.store(frame as usize, Ordering::Relaxed);
            }
            #[cfg(unix)]
            {
                let slot = cache.top_frame.get();
                if !slot.is_null() {
                    let fo_ptr = if frame.is_null() {
                        core::ptr::null_mut()
                    } else {
                        let frame_obj = unsafe { (*frame).frame_obj() };
                        frame_obj.map_or(core::ptr::null_mut(), |py| {
                            py as *const Py<FrameObject> as *mut Py<FrameObject>
                        })
                    };
                    unsafe { &*slot }.store(fo_ptr, Ordering::Relaxed);
                }
            }
        }
        cache.current_frame.swap(frame as usize, Ordering::Relaxed)
    }) as *const InterpreterFrame
}

/// Lightweight version that only writes to TLS `current_frame`, returning
/// the previous value. Does not update cross-thread top_frame (that's
/// updated by `set_current_frame` for FrameObject-based calls).
///
/// # Safety
/// A non-null frame must belong to the attached interpreter and remain live
/// until the previous pointer is restored, including during signal handlers.
#[inline(always)]
#[must_use]
pub unsafe fn set_current_frame_nosave(frame: *const InterpreterFrame) -> *const InterpreterFrame {
    FRAME_SLOT_CACHE.with(|cache| cache.current_frame.swap(frame as usize, Ordering::Relaxed))
        as *const InterpreterFrame
}

/// Get the current thread's top InterpreterFrame pointer.
/// Used by faulthandler's signal handler to start traceback walking.
#[must_use]
pub fn get_current_frame() -> *const InterpreterFrame {
    FRAME_SLOT_CACHE.with(|cache| cache.current_frame.load(Ordering::Relaxed))
        as *const InterpreterFrame
}

/// Update the current thread's exception slot atomically (no locks).
/// Called from push_exception/pop_exception/set_exception.
#[cfg(feature = "threading")]
pub(crate) fn update_thread_exception(exc: Option<PyBaseExceptionRef>) {
    let slot = CURRENT_THREAD_SLOT.with(|slot| slot.borrow().clone());
    if let Some(slot) = slot
        && !slot.is_closed()
    {
        // SAFETY: called only from the owning, attached thread. Release outside
        // the TLS borrow, since a destructor may update the exception again.
        drop(unsafe { slot.exception.swap(exc) });
    }
}

/// Collect all threads' current exceptions for sys._current_exceptions().
/// Acquires the global registry lock briefly, then reads each slot's exception atomically.
#[cfg(feature = "threading")]
pub fn get_all_current_exceptions(vm: &VirtualMachine) -> Vec<(u64, Option<PyBaseExceptionRef>)> {
    let registry = vm.state.thread_frames.lock();
    registry
        .iter()
        .map(|(id, slot)| (*id, slot.exception.load_owned()))
        .collect()
}

/// Cleanup thread slot for the current thread in `vm`'s interpreter.
/// Called at thread exit (or when leaving an interpreter permanently).
#[cfg(all(test, feature = "threading"))]
fn cleanup_current_thread_frames(vm: &VirtualMachine) {
    retire_thread_slot(vm);
    unregister_current_thread_frames(&vm.state);
}

#[cfg(feature = "threading")]
pub(crate) fn retire_thread_slot(vm: &VirtualMachine) {
    let interp_id = vm.state.interpreter_id;

    let slot_to_clean = INTERP_THREAD_SLOTS.with(|slots| slots.borrow().get(&interp_id).cloned());

    // Keep the slot registered and attached throughout Python destruction.
    // Native TLS can retain the emptied slot after its interpreter goes away.
    if let Some(slot) = &slot_to_clean {
        slot.top_iframe.store(0, Ordering::Release);
        #[cfg(unix)]
        slot.top_frame
            .store(core::ptr::null_mut(), Ordering::Release);
        #[cfg(not(unix))]
        slot.frames.lock().clear();
        slot.close(vm);
    }
}

/// Release callbacks after all other execution contexts have exited or parked
/// permanently for shutdown. Native TLS may retain the emptied slots.
#[cfg(feature = "threading")]
pub(super) fn retire_interpreter_thread_roots(vm: &VirtualMachine) {
    let slots: Vec<_> = vm.state.thread_frames.lock().values().cloned().collect();
    let mut cleanup = super::owned::Cleanup::default();
    for slot in slots {
        cleanup.run(|| slot.close(vm));
    }
    cleanup.finish();
}

#[cfg(feature = "threading")]
fn unregister_current_thread_frames(state: &PyGlobalState) {
    let thread_id = crate::stdlib::_thread::get_ident();
    let interp_id = state.interpreter_id;
    let slot_to_clean = INTERP_THREAD_SLOTS.with(|slots| slots.borrow_mut().remove(&interp_id));

    // A dying thread should not remain logically ATTACHED while its
    // thread-state slot is being removed.
    if let Some(slot) = &slot_to_clean
        && slot
            .state
            .compare_exchange(
                ThreadState::Attached as i32,
                ThreadState::Detached as i32,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    {
        slot.qsbr.offline();
        slot.shared_qsbr.offline();
    }

    // Guard against OS thread-id reuse races: only remove the registry entry
    // if it still points at this thread's own slot.
    let _removed = if let Some(slot) = &slot_to_clean {
        let mut registry = state.thread_frames.lock();
        match registry.get(&thread_id) {
            Some(registered) if Arc::ptr_eq(registered, slot) => registry.remove(&thread_id),
            _ => None,
        }
    } else {
        None
    };

    if let Some(slot) = &_removed
        && state.stop_the_world.requested.load(Ordering::Acquire)
        && thread_id != state.stop_the_world.requester_ident()
        && slot.state.load(Ordering::Relaxed) != ThreadState::Suspended as i32
    {
        // A non-requester thread disappeared while stop-the-world is pending.
        // Unblock requester countdown progress.
        state.stop_the_world.notify_thread_gone();
    }

    // If CURRENT pointed at the cleaned slot, clear it (and top-frame cache).
    CURRENT_THREAD_SLOT.with(|s| {
        let clear = match (s.borrow().as_ref(), slot_to_clean.as_ref()) {
            (Some(cur), Some(cleaned)) => Arc::ptr_eq(cur, cleaned),
            (Some(_), None) => false,
            (None, _) => false,
        };
        if clear {
            *s.borrow_mut() = None;
            #[cfg(feature = "threading")]
            FRAME_SLOT_CACHE.with(|cache| {
                cache.current_frame.store(0, Ordering::Relaxed);
                #[cfg(unix)]
                cache.top_frame.set(core::ptr::null());
                cache.top_iframe.set(core::ptr::null());
            });
            #[cfg(feature = "threading")]
            CURRENT_STOP_REQUESTED.with(|c| c.set(core::ptr::null()));
        }
    });
}

/// Snapshot this native thread's reclamation registrations before fork.
#[cfg(all(unix, feature = "threading"))]
pub(super) fn surviving_qsbr_slots() -> Vec<Arc<crate::object::qsbr::QsbrSlot>> {
    INTERP_THREAD_SLOTS.with(|slots| {
        slots
            .borrow()
            .values()
            .flat_map(|slot| [slot.qsbr.clone(), slot.shared_qsbr.clone()])
            .collect()
    })
}

/// Remove vanished threads while preserving this native thread's slot. Its
/// frames, tracing hooks and exception may belong to a saved/nested entry.
#[cfg(all(unix, feature = "threading"))]
pub(crate) fn retain_frame_slot_after_fork(state: &PyGlobalState) {
    let ident = crate::stdlib::_thread::get_ident();
    let removed = {
        let mut registry = state.thread_frames.lock();
        let current = registry.remove(&ident);
        let removed = core::mem::take(&mut *registry);
        if let Some(slot) = current {
            slot.stop_requested.store(false, Ordering::Relaxed);
            slot.state
                .store(ThreadState::Detached as i32, Ordering::Relaxed);
            registry.insert(ident, slot);
        }
        removed
    };
    let _owner = state.gc.allocation_scope();
    native_sweep(|| {
        #[expect(clippy::iter_over_hash_type, reason = "independent vanished slots")]
        for slot in removed.values() {
            // Vanished native TLS retains copied Arc counts. Empty the roots
            // explicitly; dropping this registry Arc alone cannot release them.
            unsafe {
                crate::common::lock::reinit_mutex_after_fork(&slot.trace_func);
                crate::common::lock::reinit_mutex_after_fork(&slot.profile_func);
            }
            slot.closed.store(true, Ordering::Release);
            let ctx = crate::vm::Context::genesis();
            let trace = core::mem::replace(&mut *slot.trace_func.lock(), ctx.none());
            let profile = core::mem::replace(&mut *slot.profile_func.lock(), ctx.none());
            let exception = unsafe { slot.exception.swap(None) };
            slot.qsbr.offline();
            slot.shared_qsbr.offline();
            drop((trace, profile, exception));
        }
        drop(removed);
    });
}

/// Reattach the surviving slot after child repair. Keeping its identity also
/// preserves the cached frame pointers and its QSBR registrations.
#[cfg(all(unix, feature = "threading"))]
pub fn reinit_frame_slot_after_fork(vm: &VirtualMachine) {
    retain_frame_slot_after_fork(&vm.state);
    init_thread_slot_if_needed(vm);
    attach_thread(vm);
}

/// Whether the interpreter on top of `VM_STACK` is currently ATTACHED on this
/// thread. Without the `threading` feature there is no attach state to check.
#[cfg(feature = "threading")]
fn top_slot_is_attached() -> bool {
    current_slot_is_attached()
}
#[cfg(not(feature = "threading"))]
fn top_slot_is_attached() -> bool {
    true
}

/// Which VM `with_vm` found for `obj`, and whether this thread is already
/// attached to it (see the fast path below).
enum WithVmTarget {
    /// `interp` is on top of `VM_STACK`, so this thread is already ATTACHED
    /// to it (see [`begin_interpreter_section`]'s invariant): no section
    /// switch is needed.
    AlreadyCurrent(NonNull<VirtualMachine>),
    /// `interp` owns `obj` but is not the top of `VM_STACK` (a nested,
    /// currently-detached interpreter), so a real attach/detach section is
    /// required.
    NeedsSwitch(NonNull<VirtualMachine>),
}

pub fn with_vm<F, R>(obj: &PyObject, f: F) -> Option<R>
where
    F: FnOnce(&VirtualMachine) -> R,
{
    if !callbacks_permitted() {
        return None;
    }
    let vm_owns_obj = |interp: NonNull<VirtualMachine>| {
        // SAFETY: all references in VM_STACK should be valid
        let vm = unsafe { interp.as_ref() };
        obj.gc_handle()
            .heap()
            .and_then(|heap| heap.owner)
            .is_none_or(|owner| owner == vm.state.interpreter_id)
    };
    // `with_vm` runs on every teardown of an object with a `__del__` slot or a
    // weakref callback (drop_slow_inner / try_call_finalizer / gc_state), which
    // for `__del__` objects and weakrefs collected during a GC pass means it
    // runs on essentially every such drop. The overwhelming majority of those
    // drops happen from within Python bytecode executing on this very thread,
    // i.e. `obj`'s owning interpreter is already the one on top of `VM_STACK`.
    // `begin_interpreter_section` (via `set_current_vm`) only ever needs to run
    // for the rare case where the object belongs to a *different* interpreter
    // than the one currently attached (a nested subinterpreter scenario) or no
    // interpreter is attached at all (object dropped on a thread outside any
    // VM, e.g. during shutdown or from a plain Rust thread) — in the fast case
    // we can skip it entirely and call `f` directly.
    let target = VM_STACK.with(|vms| {
        let vms = vms.borrow();
        // Fast path: at most one interpreter is ever ATTACHED per OS thread,
        // and it is always the one on top of `VM_STACK` — every push onto
        // VM_STACK (set_current_vm, VmBootstrapGuard) is paired with an attach
        // *before* the push, and every pop is paired with a detach (or a
        // re-attach of the newly-exposed top) in `end_interpreter_section`.
        // So if `obj`'s owning interpreter is the current top, this thread is
        // already attached to it and there is nothing for
        // `begin_interpreter_section` to do: no INTERP_THREAD_SLOTS lookup, no
        // Arc clone, no atomic state transition.
        // The top may still be DETACHED while the thread sits inside an
        // `allow_threads` section (a blocking call that dropped an object
        // with `__del__`); running `f` there would execute Python on a thread
        // a stop-the-world requester counts as parked, so that case takes the
        // full attach path below.
        if let Some(top) = vms.last().copied()
            && vm_owns_obj(top)
            && top_slot_is_attached()
        {
            return Some(WithVmTarget::AlreadyCurrent(top));
        }
        let interp = vms.iter().rev().copied().find(|x| vm_owns_obj(*x))?;
        Some(WithVmTarget::NeedsSwitch(interp))
    })?;
    match target {
        WithVmTarget::AlreadyCurrent(interp) => {
            // SAFETY: `interp` is (or was, at the point it was read above) the
            // top of VM_STACK for this thread, so it is valid for at least the
            // dynamic scope of the enclosing set_current_vm()/enter_vm() call,
            // which contains this whole function call.
            let vm = unsafe { interp.as_ref() };
            Some(f(vm))
        }
        WithVmTarget::NeedsSwitch(interp) => {
            // SAFETY: all references in VM_STACK should be valid, and should not be changed or moved
            // at least until this function returns and the stack unwinds to an enter_vm() call
            let vm = unsafe { interp.as_ref() };
            Some(set_current_vm(vm, || f(vm)))
        }
    }
}

#[must_use = "ThreadedVirtualMachine does nothing unless you move it to another thread and call .run()"]
#[cfg(feature = "threading")]
pub struct ThreadedVirtualMachine {
    pub(super) vm: super::owned::OwnedVm,
}

#[cfg(feature = "threading")]
impl ThreadedVirtualMachine {
    /// Enter this interpreter on the current native thread with scoped access.
    pub fn run<R>(&self, f: impl for<'vm> FnOnce(crate::embedding::Vm<'vm>) -> R) -> R {
        if self.vm.state.closed.load(Ordering::Acquire) || !self.vm.owner_is_valid() {
            return f(crate::embedding::Vm::new(&self.vm));
        }
        self.run_raw(|vm| f(crate::embedding::Vm::new(vm)))
    }

    /// Enter the raw native API on this thread.
    ///
    /// # Safety
    /// Follow [`crate::Interpreter::enter_unchecked`]'s contract.
    pub unsafe fn run_unchecked<R>(&self, f: impl FnOnce(&VirtualMachine) -> R) -> R {
        self.run_raw(f)
    }

    /// Build a scoped interpreter entry for a native thread.
    pub fn make_spawn_func<R>(
        self,
        f: impl for<'vm> FnOnce(crate::embedding::Vm<'vm>) -> R,
    ) -> impl FnOnce() -> R {
        move || self.run(f)
    }

    /// Create a `FnOnce()` that can easily be passed to a function like [`std::thread::Builder::spawn`]
    ///
    /// # Note
    ///
    /// If you return a `PyObjectRef` (or a type that contains one) from `F`, and don't `join()`
    /// on the thread this `FnOnce` runs in, there is a possibility that that thread will panic
    /// as `PyObjectRef`'s `Drop` implementation tries to run the `__del__` destructor of a
    /// Python object but finds that it's not in the context of any vm.
    pub(crate) fn make_spawn_func_raw<F, R>(self, f: F) -> impl FnOnce() -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        move || self.run_raw(f)
    }

    /// Run a function in this thread context
    ///
    /// # Note
    ///
    /// If you return a `PyObjectRef` (or a type that contains one) from `F`, and don't return the object
    /// to the parent thread and then `join()` on the `JoinHandle` (or similar), there is a possibility that
    /// the current thread will panic as `PyObjectRef`'s `Drop` implementation tries to run the `__del__`
    /// destructor of a python object but finds that it's not in the context of any vm.
    pub(crate) fn run_raw<F, R>(&self, f: F) -> R
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
        let func = self.new_thread().make_spawn_func_raw(f);
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
    /// let interpreter = rustpython_vm::Interpreter::without_stdlib(Default::default());
    /// let handle = std::thread::spawn(
    ///     interpreter.new_thread().make_spawn_func(|vm| vm.new_int(42).unwrap().unbind())
    /// );
    /// let returned = handle.join().unwrap();
    /// interpreter.enter(|vm| assert_eq!(vm.bind(&returned).unwrap().to_i64().unwrap(), 42));
    /// ```
    ///
    /// Note: this function is safe, but running the returned ThreadedVirtualMachine in the same
    /// thread context (i.e. with the same thread-local storage) doesn't have any
    /// specific guaranteed behavior.
    #[cfg(feature = "threading")]
    pub fn new_thread(&self) -> ThreadedVirtualMachine {
        assert!(
            !self.state.finalizing.load(Ordering::Acquire)
                && !self.state.admission_closed.load(Ordering::Acquire),
            "cannot create a worker during interpreter shutdown"
        );
        let global_trace = self.state.global_trace_func.lock().clone();
        let global_profile = self.state.global_profile_func.lock().clone();
        let use_tracing = global_trace.is_some() || global_profile.is_some();

        let vm = Self {
            owner_lease: core::cell::OnceCell::new(),
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
            what_event: Cell::new(None),
            tracing_depth: Cell::new(0),
            recursion_limit: self.recursion_limit.clone(),
            signal_handlers: core::cell::OnceCell::new(),
            signal_rx: core::cell::OnceCell::new(),
            repr_guards: RefCell::default(),
            state: self.state.clone(),
            initialized: self.initialized.clone(),
            recursion_depth: Cell::new(0),
            #[cfg(any(miri, target_env = "musl"))]
            native_recursion_depth: Cell::new(0),
            c_stack_soft_limit: Cell::new(Self::calculate_c_stack_soft_limit()),
            async_gen_firstiter: RefCell::new(None),
            async_gen_finalizer: RefCell::new(None),
            asyncio_running_loop: RefCell::new(None),
            asyncio_running_task: RefCell::new(None),
            context_stack: RefCell::default(),
            callable_cache: self.callable_cache.clone(),
            pending_tailcall_frame: Cell::new(None),
            pending_tailcall_owner: core::cell::UnsafeCell::new(None),
            pending_gen_resume: core::cell::UnsafeCell::new(None),
            trampoline_stack: core::cell::UnsafeCell::new(Vec::new()),
        };
        ThreadedVirtualMachine {
            vm: super::owned::OwnedVm::new(vm),
        }
    }
}

/// Access the raw VM currently entered on this native thread.
///
/// # Safety
/// Follow [`crate::Interpreter::enter_unchecked`]'s contract. The callback must
/// not leak raw references, and the thread must be attached for object access.
pub unsafe fn with_current_vm_unchecked<R>(f: impl FnOnce(&VirtualMachine) -> R) -> R {
    with_current_vm(f)
}

/// Preserve the current attachment for native code that releases it manually.
///
/// # Safety
/// Restore on this same native thread before any enclosing VM entry returns.
/// No raw Python reference may be accessed while detached.
#[cfg(feature = "threading")]
#[must_use]
pub unsafe fn save_current_thread_unchecked() -> SavedThreadState {
    save_current_thread()
}

/// Restore a manually detached native entry.
///
/// # Safety
/// The saved state's entries must all still be alive, on this native thread,
/// and this thread must not have entered any other VM since saving it.
#[cfg(feature = "threading")]
pub unsafe fn restore_current_thread_unchecked(state: SavedThreadState) {
    restore_current_thread(state);
}

#[cfg(test)]
#[cfg(all(feature = "threading", feature = "rustpython-compiler"))]
mod tests {
    use super::*;

    // Only use for runtimes exclusively owned by this test. This exercises the
    // ownership transition; the Unix fork coordinator must also stabilize locks.
    fn repair_native_owners(state: &PyGlobalState) {
        let (owners, teardowns) = unsafe { state.owner_leases.prepare_fork().repair_child() };
        state.owners.store(owners, Ordering::Relaxed);
        state.teardowns.store(teardowns, Ordering::Relaxed);
    }

    #[test]
    fn inherited_idle_owner_cannot_enter_or_decrement_surviving_owners() {
        let interpreter = crate::Interpreter::without_stdlib(Default::default());
        let idle = interpreter.new_thread();
        let nested = interpreter.new_thread();
        interpreter.enter_raw(|vm| {
            nested.run_raw(|_| repair_native_owners(&vm.state));
            assert_eq!(vm.state.owners.load(Ordering::Acquire), 2);
            assert!(matches!(
                idle.run(|vm| vm.new_int(1).map(|value| value.unbind())),
                Err(crate::embedding::Error::ForkedOwner)
            ));
            drop(idle);
            assert_eq!(vm.state.owners.load(Ordering::Acquire), 2);
            drop(nested);
            assert_eq!(vm.state.owners.load(Ordering::Acquire), 1);
        });
        assert_eq!(interpreter.finalize().unwrap(), 0);
    }

    #[test]
    fn saved_native_entries_survive_owner_repair() {
        let interpreter = crate::Interpreter::without_stdlib(Default::default());
        let other = crate::Interpreter::without_stdlib(Default::default());
        interpreter.enter_raw(|vm| {
            let saved = save_current_thread();
            other.enter_raw(|_| repair_native_owners(&vm.state));
            restore_current_thread(saved);
            assert!(is_current_attached(vm));
            assert!(vm.owner_is_valid());
            assert_eq!(vm.state.owners.load(Ordering::Acquire), 1);
        });
        assert_eq!(interpreter.finalize().unwrap(), 0);
    }

    #[test]
    fn native_destructor_panic_unregisters_before_restoring_outer_vm() {
        struct PanicDrop;
        impl Drop for PanicDrop {
            fn drop(&mut self) {
                assert!(!callbacks_permitted());
                assert!(try_with_current_vm(|_| ()).is_none());
                panic!("native destructor");
            }
        }

        let outer = crate::Interpreter::without_stdlib(Default::default());
        let interpreter = crate::Interpreter::without_stdlib(Default::default());
        let state = interpreter.enter_raw(|vm| vm.state.clone());
        let worker = interpreter.new_thread();
        let probe = PanicDrop;
        let (sender, receiver) = crate::signal::user_signal_channel();
        sender
            .send(Box::new(move |_| {
                let _ = &probe;
                Ok(())
            }))
            .unwrap();
        worker.vm.set_user_signal_channel(receiver);
        drop(sender);
        outer.enter(|vm| {
            let result = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| drop(worker)));
            assert!(result.is_err());
            assert!(callbacks_permitted());
            assert_eq!(state.teardowns.load(Ordering::Acquire), 0);
            assert_eq!(state.owners.load(Ordering::Acquire), 1);
            assert!(state.thread_frames.lock().is_empty());
            assert_eq!(vm.new_int(42).unwrap().to_i64().unwrap(), 42);
        });
        assert_eq!(interpreter.finalize().unwrap(), 0);
    }

    #[test]
    fn finalize_waits_for_the_native_tail_of_worker_destruction() {
        check_native_tail(false);
    }

    #[test]
    fn finalize_waits_for_an_inherited_stale_owners_native_tail() {
        check_native_tail(true);
    }

    fn check_native_tail(stale: bool) {
        use core::time::Duration;
        use std::sync::mpsc;

        struct PauseDrop {
            started: mpsc::Sender<()>,
            resume: std::sync::Mutex<mpsc::Receiver<()>>,
        }
        impl Drop for PauseDrop {
            fn drop(&mut self) {
                assert!(!callbacks_permitted());
                self.started.send(()).unwrap();
                let resumed = self.resume.get_mut().unwrap();
                wait_detached_from_interpreter(&|| {
                    resumed.recv_timeout(Duration::from_secs(30)).unwrap();
                });
            }
        }

        let interpreter = crate::Interpreter::without_stdlib(Default::default());
        let worker = interpreter.new_thread();
        let (started, waiting) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        let probe = PauseDrop {
            started,
            resume: std::sync::Mutex::new(resumed),
        };
        let (sender, receiver) = crate::signal::user_signal_channel();
        sender
            .send(Box::new(move |_| {
                let _ = &probe;
                Ok(())
            }))
            .unwrap();
        worker.vm.set_user_signal_channel(receiver);
        drop(sender);
        if stale {
            interpreter.enter_raw(|vm| repair_native_owners(&vm.state));
            assert!(!worker.vm.owner_is_valid());
        }
        let worker = std::thread::spawn(move || drop(worker));
        let release = scopeguard::guard(resume, |resume| {
            let _ = resume.send(());
        });
        waiting.recv_timeout(Duration::from_secs(30)).unwrap();
        let busy = interpreter.finalize().unwrap_err();
        drop(release);
        worker.join().unwrap();
        assert_eq!(busy.retry().unwrap(), 0);
    }

    #[test]
    fn finalization_defers_foreign_stale_drop_without_blocking_join() {
        struct Probe(Arc<AtomicUsize>);
        impl Drop for Probe {
            fn drop(&mut self) {
                assert!(!callbacks_permitted());
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let interpreter = crate::Interpreter::without_stdlib(Default::default());
        let worker = interpreter.new_thread();
        let count = Arc::new(AtomicUsize::new(0));
        let probe = Probe(count.clone());
        let (sender, receiver) = crate::signal::user_signal_channel();
        sender
            .send(Box::new(move |_| {
                let _ = &probe;
                Ok(())
            }))
            .unwrap();
        worker.vm.set_user_signal_channel(receiver);
        drop(sender);
        let state = interpreter.enter_raw(|vm| {
            repair_native_owners(&vm.state);
            vm.state.clone()
        });
        let permit = state.owner_leases.try_finalize(&state).unwrap();
        let (finished, joined) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            drop(worker);
            finished.send(()).unwrap();
        });
        joined
            .recv_timeout(core::time::Duration::from_secs(10))
            .unwrap();
        worker.join().unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 0);
        assert_eq!(state.teardowns.load(Ordering::Acquire), 0);
        drop(interpreter);
        drop(permit);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert_eq!(state.owners.load(Ordering::Acquire), 0);
        assert_eq!(state.teardowns.load(Ordering::Acquire), 0);
    }

    #[test]
    fn retired_thread_slots_release_callbacks_attached_and_cannot_be_refilled() {
        let interpreter = crate::Interpreter::without_stdlib(Default::default());
        let retired = interpreter.enter_raw(|vm| {
            let scope = crate::embedding::Vm::new(vm);
            scope.exec("import sys\nevents = []\nclass Callback:\n def __del__(self):\n  sys.settrace(lambda *args: None)\n  events.append(1)\n").unwrap();
            let handle = scope.eval("Callback()").unwrap().unbind();
            let callback = unsafe { handle.to_object_unchecked(vm) }.unwrap();
            let slot = current_thread_slot().unwrap();
            drop(slot.replace_profile(callback));
            drop(handle);
            cleanup_current_thread_frames(vm);
            assert!(slot.is_closed());
            assert!(vm.is_none(&slot.trace_func.lock()));
            assert!(vm.is_none(&slot.profile_func.lock()));
            assert!(slot.exception.load_owned().is_none());
            assert!(current_thread_slot().is_none());
            assert!(slot.qsbr.is_offline());
            slot
        });
        interpreter.enter(|vm| {
            assert_eq!(vm.eval("len(events)").unwrap().to_i64().unwrap(), 1);
        });
        // Keeping a retired slot in native TLS must not retain Python state.
        std::thread::spawn(move || drop(retired)).join().unwrap();
    }
}
