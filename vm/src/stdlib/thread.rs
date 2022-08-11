//! Implementation of the _thread module
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
pub(crate) use _thread::{make_module, RawRMutex};

#[pymodule]
pub(crate) mod _thread {
    use crate::{
        builtins::{PyDictRef, PyStrRef, PyTupleRef, PyTypeRef},
        convert::ToPyException,
        function::{ArgCallable, Either, FuncArgs, KwArgs, OptionalArg, PySetterValue},
        types::{Constructor, GetAttr, SetAttr},
        AsObject, Py, PyPayload, PyRef, PyResult, VirtualMachine,
    };
    use parking_lot::{
        lock_api::{RawMutex as RawMutexT, RawMutexTimed, RawReentrantMutex},
        RawMutex, RawThreadId,
    };
    use std::{cell::RefCell, fmt, thread, time::Duration};
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
        #[pyarg(any, default = "true")]
        blocking: bool,
        #[pyarg(any, default = "Either::A(-1.0)")]
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
            format!(
                "<{} {} object at {:#x}>",
                status,
                $zelf.class().name(),
                $zelf.get_id()
            )
        }};
    }

    #[pyattr(name = "LockType")]
    #[pyclass(module = "thread", name = "lock")]
    #[derive(PyPayload)]
    struct Lock {
        mu: RawMutex,
    }

    impl fmt::Debug for Lock {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.pad("Lock")
        }
    }

    #[pyclass(with(Constructor))]
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
                return Err(vm.new_runtime_error("release unlocked lock".to_owned()));
            }
            unsafe { self.mu.unlock() };
            Ok(())
        }

        #[pymethod(magic)]
        fn exit(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.release(vm)
        }

        #[pymethod]
        fn locked(&self) -> bool {
            self.mu.is_locked()
        }

        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>) -> String {
            repr_lock_impl!(zelf)
        }
    }

    impl Constructor for Lock {
        type Args = FuncArgs;
        fn py_new(_cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot create '_thread.lock' instances".to_owned()))
        }
    }

    pub type RawRMutex = RawReentrantMutex<RawMutex, RawThreadId>;
    #[pyattr]
    #[pyclass(module = "thread", name = "RLock")]
    #[derive(PyPayload)]
    struct RLock {
        mu: RawRMutex,
    }

    impl fmt::Debug for RLock {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.pad("RLock")
        }
    }

    #[pyclass]
    impl RLock {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            RLock {
                mu: RawRMutex::INIT,
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }

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
                return Err(vm.new_runtime_error("release unlocked lock".to_owned()));
            }
            unsafe { self.mu.unlock() };
            Ok(())
        }

        #[pymethod]
        fn _is_owned(&self) -> bool {
            self.mu.is_owned_by_current_thread()
        }

        #[pymethod(magic)]
        fn exit(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.release(vm)
        }

        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>) -> String {
            repr_lock_impl!(zelf)
        }
    }

    #[pyfunction]
    fn get_ident() -> u64 {
        thread_to_id(&thread::current())
    }

    fn thread_to_id(t: &thread::Thread) -> u64 {
        use std::hash::{Hash, Hasher};
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
    fn allocate_lock() -> Lock {
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
        SENTINELS.with(|sents| {
            for lock in sents.replace(Default::default()) {
                if lock.mu.is_locked() {
                    unsafe { lock.mu.unlock() };
                }
            }
        });
        vm.state.thread_count.fetch_sub(1);
    }

    #[pyfunction]
    fn exit(vm: &VirtualMachine) -> PyResult {
        Err(vm.new_exception_empty(vm.ctx.exceptions.system_exit.to_owned()))
    }

    thread_local!(static SENTINELS: RefCell<Vec<PyRef<Lock>>> = RefCell::default());

    #[pyfunction]
    fn _set_sentinel(vm: &VirtualMachine) -> PyRef<Lock> {
        let lock = Lock { mu: RawMutex::INIT }.into_ref(vm);
        SENTINELS.with(|sents| sents.borrow_mut().push(lock.clone()));
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

    #[pyattr]
    #[pyclass(module = "thread", name = "_local")]
    #[derive(Debug, PyPayload)]
    struct Local {
        data: ThreadLocal<PyDictRef>,
    }

    #[pyclass(with(GetAttr, SetAttr), flags(BASETYPE))]
    impl Local {
        fn ldict(&self, vm: &VirtualMachine) -> PyDictRef {
            self.data.get_or(|| vm.ctx.new_dict()).clone()
        }

        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Local {
                data: ThreadLocal::new(),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    impl GetAttr for Local {
        fn getattro(zelf: &Py<Self>, attr: PyStrRef, vm: &VirtualMachine) -> PyResult {
            let ldict = zelf.ldict(vm);
            if attr.as_str() == "__dict__" {
                Ok(ldict.into())
            } else {
                zelf.as_object()
                    .generic_getattr_opt(attr.clone(), Some(ldict), vm)?
                    .ok_or_else(|| {
                        vm.new_attribute_error(format!(
                            "{} has no attribute '{}'",
                            zelf.as_object(),
                            attr
                        ))
                    })
            }
        }
    }

    impl SetAttr for Local {
        fn setattro(
            zelf: &crate::Py<Self>,
            attr: PyStrRef,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if attr.as_str() == "__dict__" {
                Err(vm.new_attribute_error(format!(
                    "{} attribute '__dict__' is read-only",
                    zelf.as_object()
                )))
            } else {
                let dict = zelf.ldict(vm);
                if let PySetterValue::Assign(value) = value {
                    dict.set_item(&*attr, value, vm)?;
                } else {
                    dict.del_item(&*attr, vm)?;
                }
                Ok(())
            }
        }
    }
}
