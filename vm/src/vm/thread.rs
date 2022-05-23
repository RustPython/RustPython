use crate::{AsObject, PyObject, VirtualMachine};
use itertools::Itertools;
use std::{cell::RefCell, ptr::NonNull, thread_local};

thread_local! {
    pub(super) static VM_STACK: RefCell<Vec<NonNull<VirtualMachine>>> = Vec::with_capacity(1).into();
}

pub fn enter_vm<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    VM_STACK.with(|vms| {
        vms.borrow_mut().push(vm.into());
        let ret = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        vms.borrow_mut().pop();
        ret.unwrap_or_else(|e| std::panic::resume_unwind(e))
    })
}

pub fn with_vm<F, R>(obj: &PyObject, f: F) -> Option<R>
where
    F: Fn(&VirtualMachine) -> R,
{
    let vm_owns_obj = |intp: NonNull<VirtualMachine>| {
        // SAFETY: all references in VM_STACK should be valid
        let vm = unsafe { intp.as_ref() };
        obj.fast_isinstance(vm.ctx.types.object_type)
    };
    VM_STACK.with(|vms| {
        let intp = match vms.borrow().iter().copied().exactly_one() {
            Ok(x) => {
                debug_assert!(vm_owns_obj(x));
                x
            }
            Err(mut others) => others.find(|x| vm_owns_obj(*x))?,
        };
        // SAFETY: all references in VM_STACK should be valid, and should not be changed or moved
        // at least until this function returns and the stack unwinds to an enter_vm() call
        let vm = unsafe { intp.as_ref() };
        Some(f(vm))
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
    pub fn run<F, R>(self, f: F) -> R
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
        F: FnOnce(&VirtualMachine) -> R,
        F: Send + 'static,
        R: Send + 'static,
    {
        let thread = self.new_thread();
        std::thread::spawn(|| thread.run(f))
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
        use std::cell::Cell;
        let vm = VirtualMachine {
            builtins: self.builtins.clone(),
            sys_module: self.sys_module.clone(),
            ctx: self.ctx.clone(),
            frames: RefCell::new(vec![]),
            wasm_id: self.wasm_id.clone(),
            exceptions: RefCell::default(),
            import_func: self.import_func.clone(),
            profile_func: RefCell::new(self.ctx.none()),
            trace_func: RefCell::new(self.ctx.none()),
            use_tracing: Cell::new(false),
            recursion_limit: self.recursion_limit.clone(),
            signal_handlers: None,
            signal_rx: None,
            repr_guards: RefCell::default(),
            state: self.state.clone(),
            initialized: self.initialized,
            recursion_depth: Cell::new(0),
        };
        ThreadedVirtualMachine { vm }
    }
}
