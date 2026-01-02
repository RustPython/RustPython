//! Implementation of the _thread module
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
pub(crate) use _thread::{RawRMutex, make_module};

#[pymodule]
pub(crate) mod _thread {
    use crate::{
        AsObject, Py, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyStr, PyTupleRef, PyType, PyTypeRef},
        convert::ToPyException,
        function::{ArgCallable, Either, FuncArgs, KwArgs, OptionalArg, PySetterValue},
        types::{Constructor, GetAttr, Representable, SetAttr},
    };
    use alloc::fmt;
    use core::{cell::RefCell, time::Duration};
    use crossbeam_utils::atomic::AtomicCell;
    use parking_lot::{
        RawMutex, RawThreadId,
        lock_api::{RawMutex as RawMutexT, RawMutexTimed, RawReentrantMutex},
    };
    use std::thread;
    use thread_local::ThreadLocal;

    // PYTHREAD_NAME: show current thread name
    pub const PYTHREAD_NAME: Option<&str> = {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                Some("nt")
            } else if #[cfg(unix)] {
                Some("pthread")
            } else if #[cfg(any(target_os = "solaris", target_os = "illumos"))] {
                Some("solaris")
            } else {
                None
            }
        }
    };

    // TIMEOUT_MAX_IN_MICROSECONDS is a value in microseconds
    #[cfg(not(target_os = "windows"))]
    const TIMEOUT_MAX_IN_MICROSECONDS: i64 = i64::MAX / 1_000;

    #[cfg(target_os = "windows")]
    const TIMEOUT_MAX_IN_MICROSECONDS: i64 = 0xffffffff * 1_000;

    // this is a value in seconds
    #[pyattr]
    const TIMEOUT_MAX: f64 = (TIMEOUT_MAX_IN_MICROSECONDS / 1_000_000) as f64;

    #[pyattr]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.runtime_error.to_owned()
    }

    #[derive(FromArgs)]
    struct AcquireArgs {
        #[pyarg(any, default = true)]
        blocking: bool,
        #[pyarg(any, default = Either::A(-1.0))]
        timeout: Either<f64, i64>,
    }

    macro_rules! acquire_lock_impl {
        ($mu:expr, $args:expr, $vm:expr) => {{
            let (mu, args, vm) = ($mu, $args, $vm);
            let timeout = match args.timeout {
                Either::A(f) => f,
                Either::B(i) => i as f64,
            };
            match args.blocking {
                true if timeout == -1.0 => {
                    mu.lock();
                    Ok(true)
                }
                true if timeout < 0.0 => {
                    Err(vm.new_value_error("timeout value must be positive".to_owned()))
                }
                true => {
                    // modified from std::time::Duration::from_secs_f64 to avoid a panic.
                    // TODO: put this in the Duration::try_from_object impl, maybe?
                    let nanos = timeout * 1_000_000_000.0;
                    if timeout > TIMEOUT_MAX as f64 || nanos < 0.0 || !nanos.is_finite() {
                        return Err(vm.new_overflow_error(
                            "timestamp too large to convert to Rust Duration".to_owned(),
                        ));
                    }

                    Ok(mu.try_lock_for(Duration::from_secs_f64(timeout)))
                }
                false if timeout != -1.0 => Err(vm
                    .new_value_error("can't specify a timeout for a non-blocking call".to_owned())),
                false => Ok(mu.try_lock()),
            }
        }};
    }
    macro_rules! repr_lock_impl {
        ($zelf:expr) => {{
            let status = if $zelf.mu.is_locked() {
                "locked"
            } else {
                "unlocked"
            };
            Ok(format!(
                "<{} {} object at {:#x}>",
                status,
                $zelf.class().name(),
                $zelf.get_id()
            ))
        }};
    }

    #[pyattr(name = "LockType")]
    #[pyclass(module = "thread", name = "lock")]
    #[derive(PyPayload)]
    struct Lock {
        mu: RawMutex,
    }

    impl fmt::Debug for Lock {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("Lock")
        }
    }

    #[pyclass(with(Constructor, Representable))]
    impl Lock {
        #[pymethod]
        #[pymethod(name = "acquire_lock")]
        #[pymethod(name = "__enter__")]
        fn acquire(&self, args: AcquireArgs, vm: &VirtualMachine) -> PyResult<bool> {
            acquire_lock_impl!(&self.mu, args, vm)
        }
        #[pymethod]
        #[pymethod(name = "release_lock")]
        fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
            if !self.mu.is_locked() {
                return Err(vm.new_runtime_error("release unlocked lock"));
            }
            unsafe { self.mu.unlock() };
            Ok(())
        }

        #[pymethod]
        fn _at_fork_reinit(&self, _vm: &VirtualMachine) -> PyResult<()> {
            if self.mu.is_locked() {
                unsafe {
                    self.mu.unlock();
                };
            }
            // Casting to AtomicCell is as unsafe as CPython code.
            // Using AtomicCell will prevent compiler optimizer move it to somewhere later unsafe place.
            // It will be not under the cell anymore after init call.

            let new_mut = RawMutex::INIT;
            unsafe {
                let old_mutex: &AtomicCell<RawMutex> = core::mem::transmute(&self.mu);
                old_mutex.swap(new_mut);
            }

            Ok(())
        }

        #[pymethod]
        fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.release(vm)
        }

        #[pymethod]
        fn locked(&self) -> bool {
            self.mu.is_locked()
        }
    }

    impl Constructor for Lock {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Err(vm.new_type_error("cannot create '_thread.lock' instances"))
        }
    }

    impl Representable for Lock {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            repr_lock_impl!(zelf)
        }
    }

    pub type RawRMutex = RawReentrantMutex<RawMutex, RawThreadId>;
    #[pyattr]
    #[pyclass(module = "thread", name = "RLock")]
    #[derive(PyPayload)]
    struct RLock {
        mu: RawRMutex,
        count: core::sync::atomic::AtomicUsize,
    }

    impl fmt::Debug for RLock {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("RLock")
        }
    }

    #[pyclass(with(Representable))]
    impl RLock {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Self {
                mu: RawRMutex::INIT,
                count: core::sync::atomic::AtomicUsize::new(0),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }

        #[pymethod]
        #[pymethod(name = "acquire_lock")]
        #[pymethod(name = "__enter__")]
        fn acquire(&self, args: AcquireArgs, vm: &VirtualMachine) -> PyResult<bool> {
            let result = acquire_lock_impl!(&self.mu, args, vm)?;
            if result {
                self.count
                    .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
            Ok(result)
        }
        #[pymethod]
        #[pymethod(name = "release_lock")]
        fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
            if !self.mu.is_locked() {
                return Err(vm.new_runtime_error("release unlocked lock"));
            }
            debug_assert!(
                self.count.load(core::sync::atomic::Ordering::Relaxed) > 0,
                "RLock count underflow"
            );
            self.count
                .fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
            unsafe { self.mu.unlock() };
            Ok(())
        }

        #[pymethod]
        fn _at_fork_reinit(&self, _vm: &VirtualMachine) -> PyResult<()> {
            if self.mu.is_locked() {
                unsafe {
                    self.mu.unlock();
                };
            }
            self.count.store(0, core::sync::atomic::Ordering::Relaxed);
            let new_mut = RawRMutex::INIT;

            let old_mutex: AtomicCell<&RawRMutex> = AtomicCell::new(&self.mu);
            old_mutex.swap(&new_mut);

            Ok(())
        }

        #[pymethod]
        fn _is_owned(&self) -> bool {
            self.mu.is_owned_by_current_thread()
        }

        #[pymethod]
        fn _recursion_count(&self) -> usize {
            if self.mu.is_owned_by_current_thread() {
                self.count.load(core::sync::atomic::Ordering::Relaxed)
            } else {
                0
            }
        }

        #[pymethod]
        fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.release(vm)
        }
    }

    impl Representable for RLock {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            repr_lock_impl!(zelf)
        }
    }

    #[pyfunction]
    fn get_ident() -> u64 {
        thread_to_id(&thread::current())
    }

    fn thread_to_id(t: &thread::Thread) -> u64 {
        use core::hash::{Hash, Hasher};
        struct U64Hash {
            v: Option<u64>,
        }
        impl Hasher for U64Hash {
            fn write(&mut self, _: &[u8]) {
                unreachable!()
            }
            fn write_u64(&mut self, i: u64) {
                self.v = Some(i);
            }
            fn finish(&self) -> u64 {
                self.v.expect("should have written a u64")
            }
        }
        // TODO: use id.as_u64() once it's stable, until then, ThreadId is just a wrapper
        // around NonZeroU64, so this should work (?)
        let mut h = U64Hash { v: None };
        t.id().hash(&mut h);
        h.finish()
    }

    #[pyfunction]
    const fn allocate_lock() -> Lock {
        Lock { mu: RawMutex::INIT }
    }

    #[pyfunction]
    fn start_new_thread(
        func: ArgCallable,
        args: PyTupleRef,
        kwargs: OptionalArg<PyDictRef>,
        vm: &VirtualMachine,
    ) -> PyResult<u64> {
        let args = FuncArgs::new(
            args.to_vec(),
            kwargs
                .map_or_else(Default::default, |k| k.to_attributes(vm))
                .into_iter()
                .map(|(k, v)| (k.as_str().to_owned(), v))
                .collect::<KwArgs>(),
        );
        let mut thread_builder = thread::Builder::new();
        let stacksize = vm.state.stacksize.load();
        if stacksize != 0 {
            thread_builder = thread_builder.stack_size(stacksize);
        }
        thread_builder
            .spawn(
                vm.new_thread()
                    .make_spawn_func(move |vm| run_thread(func, args, vm)),
            )
            .map(|handle| {
                vm.state.thread_count.fetch_add(1);
                thread_to_id(handle.thread())
            })
            .map_err(|err| err.to_pyexception(vm))
    }

    fn run_thread(func: ArgCallable, args: FuncArgs, vm: &VirtualMachine) {
        match func.invoke(args, vm) {
            Ok(_obj) => {}
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.system_exit) => {}
            Err(exc) => {
                vm.run_unraisable(
                    exc,
                    Some("Exception ignored in thread started by".to_owned()),
                    func.into(),
                );
            }
        }
        for lock in SENTINELS.take() {
            if lock.mu.is_locked() {
                unsafe { lock.mu.unlock() };
            }
        }
        vm.state.thread_count.fetch_sub(1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[pyfunction]
    fn interrupt_main(signum: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<()> {
        crate::signal::set_interrupt_ex(signum.unwrap_or(libc::SIGINT), vm)
    }

    #[pyfunction]
    fn exit(vm: &VirtualMachine) -> PyResult {
        Err(vm.new_exception_empty(vm.ctx.exceptions.system_exit.to_owned()))
    }

    thread_local!(static SENTINELS: RefCell<Vec<PyRef<Lock>>> = const { RefCell::new(Vec::new()) });

    #[pyfunction]
    fn _set_sentinel(vm: &VirtualMachine) -> PyRef<Lock> {
        let lock = Lock { mu: RawMutex::INIT }.into_ref(&vm.ctx);
        SENTINELS.with_borrow_mut(|sentinels| sentinels.push(lock.clone()));
        lock
    }

    #[pyfunction]
    fn stack_size(size: OptionalArg<usize>, vm: &VirtualMachine) -> usize {
        let size = size.unwrap_or(0);
        // TODO: do validation on this to make sure it's not too small
        vm.state.stacksize.swap(size)
    }

    #[pyfunction]
    fn _count(vm: &VirtualMachine) -> usize {
        vm.state.thread_count.load()
    }

    /// ExceptHookArgs - simple class to hold exception hook arguments
    /// This allows threading.py to import _excepthook and _ExceptHookArgs from _thread
    #[pyattr]
    #[pyclass(module = "_thread", name = "_ExceptHookArgs")]
    #[derive(Debug, PyPayload)]
    struct ExceptHookArgs {
        exc_type: crate::PyObjectRef,
        exc_value: crate::PyObjectRef,
        exc_traceback: crate::PyObjectRef,
        thread: crate::PyObjectRef,
    }

    #[pyclass(with(Constructor))]
    impl ExceptHookArgs {
        #[pygetset]
        fn exc_type(&self) -> crate::PyObjectRef {
            self.exc_type.clone()
        }

        #[pygetset]
        fn exc_value(&self) -> crate::PyObjectRef {
            self.exc_value.clone()
        }

        #[pygetset]
        fn exc_traceback(&self) -> crate::PyObjectRef {
            self.exc_traceback.clone()
        }

        #[pygetset]
        fn thread(&self) -> crate::PyObjectRef {
            self.thread.clone()
        }
    }

    impl Constructor for ExceptHookArgs {
        // Takes a single iterable argument like namedtuple
        type Args = (crate::PyObjectRef,);

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            // Convert the argument to a list/tuple and extract elements
            let seq: Vec<crate::PyObjectRef> = args.0.try_to_value(vm)?;
            if seq.len() != 4 {
                return Err(vm.new_type_error(format!(
                    "_ExceptHookArgs expected 4 arguments, got {}",
                    seq.len()
                )));
            }
            Ok(Self {
                exc_type: seq[0].clone(),
                exc_value: seq[1].clone(),
                exc_traceback: seq[2].clone(),
                thread: seq[3].clone(),
            })
        }
    }

    /// Handle uncaught exception in Thread.run()
    #[pyfunction]
    fn _excepthook(args: crate::PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Type check: args must be _ExceptHookArgs
        let args = args.downcast::<ExceptHookArgs>().map_err(|_| {
            vm.new_type_error(
                "_thread._excepthook argument type must be _ExceptHookArgs".to_owned(),
            )
        })?;

        let exc_type = args.exc_type.clone();
        let exc_value = args.exc_value.clone();
        let exc_traceback = args.exc_traceback.clone();
        let thread = args.thread.clone();

        // Silently ignore SystemExit (identity check)
        if exc_type.is(vm.ctx.exceptions.system_exit.as_ref()) {
            return Ok(());
        }

        // Get stderr - fall back to thread._stderr if sys.stderr is None
        let file = match vm.sys_module.get_attr("stderr", vm) {
            Ok(stderr) if !vm.is_none(&stderr) => stderr,
            _ => {
                if vm.is_none(&thread) {
                    // do nothing if sys.stderr is None and thread is None
                    return Ok(());
                }
                let thread_stderr = thread.get_attr("_stderr", vm)?;
                if vm.is_none(&thread_stderr) {
                    // do nothing if sys.stderr is None and sys.stderr was None
                    // when the thread was created
                    return Ok(());
                }
                thread_stderr
            }
        };

        // Print "Exception in thread {thread.name}:"
        let thread_name = if !vm.is_none(&thread) {
            thread
                .get_attr("name", vm)
                .ok()
                .and_then(|n| n.str(vm).ok())
                .map(|s| s.as_str().to_owned())
        } else {
            None
        };
        let name = thread_name.unwrap_or_else(|| format!("{}", get_ident()));

        let _ = vm.call_method(
            &file,
            "write",
            (format!("Exception in thread {}:\n", name),),
        );

        // Display the traceback
        if let Ok(traceback_mod) = vm.import("traceback", 0)
            && let Ok(print_exc) = traceback_mod.get_attr("print_exception", vm)
        {
            use crate::function::KwArgs;
            let kwargs: KwArgs = vec![("file".to_owned(), file.clone())]
                .into_iter()
                .collect();
            let _ = print_exc.call_with_args(
                crate::function::FuncArgs::new(vec![exc_type, exc_value, exc_traceback], kwargs),
                vm,
            );
        }

        // Flush file
        let _ = vm.call_method(&file, "flush", ());
        Ok(())
    }

    #[pyattr]
    #[pyclass(module = "thread", name = "_local")]
    #[derive(Debug, PyPayload)]
    struct Local {
        data: ThreadLocal<PyDictRef>,
    }

    #[pyclass(with(GetAttr, SetAttr), flags(BASETYPE))]
    impl Local {
        fn l_dict(&self, vm: &VirtualMachine) -> PyDictRef {
            self.data.get_or(|| vm.ctx.new_dict()).clone()
        }

        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Self {
                data: ThreadLocal::new(),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    impl GetAttr for Local {
        fn getattro(zelf: &Py<Self>, attr: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            let l_dict = zelf.l_dict(vm);
            if attr.as_str() == "__dict__" {
                Ok(l_dict.into())
            } else {
                zelf.as_object()
                    .generic_getattr_opt(attr, Some(l_dict), vm)?
                    .ok_or_else(|| {
                        vm.new_attribute_error(format!(
                            "{} has no attribute '{}'",
                            zelf.class().name(),
                            attr
                        ))
                    })
            }
        }
    }

    impl SetAttr for Local {
        fn setattro(
            zelf: &Py<Self>,
            attr: &Py<PyStr>,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if attr.as_str() == "__dict__" {
                Err(vm.new_attribute_error(format!(
                    "{} attribute '__dict__' is read-only",
                    zelf.class().name()
                )))
            } else {
                let dict = zelf.l_dict(vm);
                if let PySetterValue::Assign(value) = value {
                    dict.set_item(attr, value, vm)?;
                } else {
                    dict.del_item(attr, vm)?;
                }
                Ok(())
            }
        }
    }
}
