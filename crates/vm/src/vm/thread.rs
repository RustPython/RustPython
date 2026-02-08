use crate::frame::Frame;
#[cfg(feature = "threading")]
use crate::frame::FrameRef;
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

/// Type for current frame slot - shared between threads for sys._current_frames()
/// Stores the full frame stack so faulthandler can dump complete tracebacks
/// for all threads.
#[cfg(feature = "threading")]
pub type CurrentFrameSlot = Arc<parking_lot::Mutex<Vec<FrameRef>>>;

thread_local! {
    pub(super) static VM_STACK: RefCell<Vec<NonNull<VirtualMachine>>> = Vec::with_capacity(1).into();

    pub(crate) static COROUTINE_ORIGIN_TRACKING_DEPTH: Cell<u32> = const { Cell::new(0) };

    /// Current thread's frame slot for sys._current_frames()
    #[cfg(feature = "threading")]
    static CURRENT_FRAME_SLOT: RefCell<Option<CurrentFrameSlot>> = const { RefCell::new(None) };

    /// Current top frame for signal-safe traceback walking.
    /// Mirrors `PyThreadState.current_frame`. Read by faulthandler's signal
    /// handler to dump tracebacks without accessing RefCell or locks.
    /// Uses AtomicPtr for async-signal-safety (signal handlers may read this
    /// while the owning thread is writing).
    pub(crate) static CURRENT_FRAME: AtomicPtr<Frame> =
        const { AtomicPtr::new(core::ptr::null_mut()) };
}

scoped_tls::scoped_thread_local!(static VM_CURRENT: VirtualMachine);

pub fn with_current_vm<R>(f: impl FnOnce(&VirtualMachine) -> R) -> R {
    if !VM_CURRENT.is_set() {
        panic!("call with_current_vm() but VM_CURRENT is null");
    }
    VM_CURRENT.with(f)
}

pub fn enter_vm<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    VM_STACK.with(|vms| {
        vms.borrow_mut().push(vm.into());

        // Initialize frame slot for this thread if not already done
        #[cfg(feature = "threading")]
        init_frame_slot_if_needed(vm);

        scopeguard::defer! { vms.borrow_mut().pop(); }
        VM_CURRENT.set(vm, f)
    })
}

/// Initialize frame slot for current thread if not already initialized.
/// Called automatically by enter_vm().
#[cfg(feature = "threading")]
fn init_frame_slot_if_needed(vm: &VirtualMachine) {
    CURRENT_FRAME_SLOT.with(|slot| {
        if slot.borrow().is_none() {
            let thread_id = crate::stdlib::thread::get_ident();
            let new_slot = Arc::new(parking_lot::Mutex::new(Vec::new()));
            vm.state
                .thread_frames
                .lock()
                .insert(thread_id, new_slot.clone());
            *slot.borrow_mut() = Some(new_slot);
        }
    });
}

/// Push a frame onto the current thread's shared frame stack.
/// Called when a new frame is entered.
#[cfg(feature = "threading")]
pub fn push_thread_frame(frame: FrameRef) {
    CURRENT_FRAME_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            s.lock().push(frame);
        }
    });
}

/// Pop a frame from the current thread's shared frame stack.
/// Called when a frame is exited.
#[cfg(feature = "threading")]
pub fn pop_thread_frame() {
    CURRENT_FRAME_SLOT.with(|slot| {
        if let Some(s) = slot.borrow().as_ref() {
            s.lock().pop();
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
pub fn get_current_frame() -> *const Frame {
    CURRENT_FRAME.with(|c| c.load(Ordering::Relaxed) as *const Frame)
}

/// Cleanup frame tracking for the current thread. Called at thread exit.
#[cfg(feature = "threading")]
pub fn cleanup_current_thread_frames(vm: &VirtualMachine) {
    let thread_id = crate::stdlib::thread::get_ident();
    vm.state.thread_frames.lock().remove(&thread_id);
    CURRENT_FRAME_SLOT.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Reinitialize frame slot after fork. Called in child process.
/// Creates a fresh slot and registers it for the current thread,
/// preserving the current thread's frames from `vm.frames`.
#[cfg(feature = "threading")]
pub fn reinit_frame_slot_after_fork(vm: &VirtualMachine) {
    let current_ident = crate::stdlib::thread::get_ident();
    // Preserve the current thread's frames across fork
    let current_frames: Vec<FrameRef> = vm.frames.borrow().clone();
    let new_slot = Arc::new(parking_lot::Mutex::new(current_frames));

    // After fork, only the current thread exists. If the lock was held by
    // another thread during fork, force unlock it.
    let mut registry = match vm.state.thread_frames.try_lock() {
        Some(guard) => guard,
        None => {
            // SAFETY: After fork in child process, only the current thread
            // exists. The lock holder no longer exists.
            unsafe { vm.state.thread_frames.force_unlock() };
            vm.state.thread_frames.lock()
        }
    };
    registry.clear();
    registry.insert(current_ident, new_slot.clone());
    drop(registry);

    // Update thread-local to point to the new slot
    CURRENT_FRAME_SLOT.with(|s| {
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
            wasm_id: self.wasm_id.clone(),
            exceptions: RefCell::default(),
            import_func: self.import_func.clone(),
            importlib: self.importlib.clone(),
            profile_func: RefCell::new(global_profile.unwrap_or_else(|| self.ctx.none())),
            trace_func: RefCell::new(global_trace.unwrap_or_else(|| self.ctx.none())),
            use_tracing: Cell::new(use_tracing),
            recursion_limit: self.recursion_limit.clone(),
            signal_handlers: None,
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
        };
        ThreadedVirtualMachine { vm }
    }
}
