//! Implementation of the _thread module
#[cfg(unix)]
pub(crate) use _thread::after_fork_child;
pub use _thread::get_ident;
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
pub(crate) use _thread::{
    CurrentFrameSlot, HandleEntry, RawRMutex, ShutdownEntry, get_all_current_frames,
    init_main_thread_ident, module_def,
};

#[pymodule]
pub(crate) mod _thread {
    use crate::{
        AsObject, Py, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyStr, PyTupleRef, PyType, PyTypeRef, PyUtf8StrRef},
        common::wtf8::Wtf8Buf,
        frame::FrameRef,
        function::{ArgCallable, FuncArgs, KwArgs, OptionalArg, PySetterValue, TimeoutSeconds},
        types::{Constructor, GetAttr, Representable, SetAttr},
    };
    use alloc::{
        fmt,
        sync::{Arc, Weak},
    };
    use core::{cell::RefCell, time::Duration};
    use parking_lot::{
        RawMutex, RawThreadId,
        lock_api::{RawMutex as RawMutexT, RawMutexTimed, RawReentrantMutex},
    };
    #[cfg(any(unix, windows))]
    use rustpython_host_env::thread as host_thread;
    use rustpython_common::str::levenshtein::{MOVE_COST, levenshtein_distance};
    use std::thread;

    // PYTHREAD_NAME: show current thread name
    pub(crate) const PYTHREAD_NAME: Option<&str> = cfg_select! {
        windows => Some("nt"),
        unix => Some("pthread"),
        any(target_os = "solaris", target_os = "illumos") => Some("solaris"),
        _ => None,
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
        #[pyarg(any, default = TimeoutSeconds::new(-1.0))]
        timeout: TimeoutSeconds,
    }

    macro_rules! acquire_lock_impl {
        ($mu:expr, $args:expr, $vm:expr) => {{
            let (mu, args, vm) = ($mu, $args, $vm);
            let timeout = args.timeout.to_secs_f64();
            match args.blocking {
                true if timeout == -1.0 => {
                    vm.allow_threads(|| mu.lock());
                    Ok(true)
                }
                true if timeout < 0.0 => {
                    Err(vm
                        .new_value_error("timeout value must be a non-negative number".to_owned()))
                }
                true => {
                    if timeout > TIMEOUT_MAX {
                        return Err(vm.new_overflow_error("timeout value is too large".to_owned()));
                    }

                    Ok(vm.allow_threads(|| mu.try_lock_for(Duration::from_secs_f64(timeout))))
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
    #[pyattr(name = "lock")]
    #[pyclass(module = "_thread", name = "lock")]
    #[derive(PyPayload)]
    struct Lock {
        mu: RawMutex,
    }

    impl fmt::Debug for Lock {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("Lock")
        }
    }

    #[pyclass(with(Constructor, Representable), flags(HAS_WEAKREF))]
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

        #[cfg(unix)]
        #[pymethod]
        fn _at_fork_reinit(&self, _vm: &VirtualMachine) -> PyResult<()> {
            // Overwrite lock state to unlocked. Do NOT call unlock() here —
            // after fork(), unlock_slow() would try to unpark stale waiters.
            unsafe { rustpython_common::lock::zero_reinit_after_fork(&self.mu) };
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
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self { mu: RawMutex::INIT })
        }
    }

    impl Representable for Lock {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            repr_lock_impl!(zelf)
        }
    }

    pub(crate) type RawRMutex = RawReentrantMutex<RawMutex, RawThreadId>;
    #[pyattr]
    #[pyclass(module = "_thread", name = "RLock")]
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

    #[pyclass(with(Representable), flags(BASETYPE, HAS_WEAKREF))]
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
            if self.mu.is_owned_by_current_thread() {
                // Re-entrant acquisition: just increment our count.
                // parking_lot stays at 1 level; we track recursion ourselves.
                self.count
                    .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                return Ok(true);
            }
            let result = acquire_lock_impl!(&self.mu, args, vm)?;
            if result {
                self.count.store(1, core::sync::atomic::Ordering::Relaxed);
            }
            Ok(result)
        }
        #[pymethod]
        #[pymethod(name = "release_lock")]
        fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
            if !self.mu.is_owned_by_current_thread() {
                return Err(vm.new_runtime_error("cannot release un-acquired lock"));
            }
            let prev = self
                .count
                .fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
            debug_assert!(prev > 0, "RLock count underflow");
            if prev == 1 {
                unsafe { self.mu.unlock() };
            }
            Ok(())
        }

        #[cfg(unix)]
        #[pymethod]
        fn _at_fork_reinit(&self, _vm: &VirtualMachine) -> PyResult<()> {
            // Overwrite lock state to unlocked. Do NOT call unlock() here —
            // after fork(), unlock_slow() would try to unpark stale waiters.
            self.count.store(0, core::sync::atomic::Ordering::Relaxed);
            unsafe { rustpython_common::lock::zero_reinit_after_fork(&self.mu) };
            Ok(())
        }

        #[pymethod]
        fn locked(&self) -> bool {
            self.mu.is_locked()
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
        fn _release_save(&self, vm: &VirtualMachine) -> PyResult<(usize, u64)> {
            if !self.mu.is_owned_by_current_thread() {
                return Err(vm.new_runtime_error("cannot release un-acquired lock"));
            }
            let count = self.count.swap(0, core::sync::atomic::Ordering::Relaxed);
            debug_assert!(count > 0, "RLock count underflow");
            unsafe { self.mu.unlock() };
            Ok((count, current_thread_id()))
        }

        #[pymethod]
        fn _acquire_restore(&self, state: PyTupleRef, vm: &VirtualMachine) -> PyResult<()> {
            let [count_obj, owner_obj] = state.as_slice() else {
                return Err(
                    vm.new_type_error("_acquire_restore() argument 1 must be a 2-item tuple")
                );
            };
            let count: usize = count_obj.clone().try_into_value(vm)?;
            let _owner: u64 = owner_obj.clone().try_into_value(vm)?;
            if count == 0 {
                return Ok(());
            }
            vm.allow_threads(|| self.mu.lock());
            self.count
                .store(count, core::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        #[pymethod]
        fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.release(vm)
        }
    }

    impl Representable for RLock {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let count = zelf.count.load(core::sync::atomic::Ordering::Relaxed);
            let status = if zelf.mu.is_locked() {
                "locked"
            } else {
                "unlocked"
            };
            Ok(format!(
                "<{} {} object count={} at {:#x}>",
                status,
                zelf.class().name(),
                count,
                zelf.get_id()
            ))
        }
    }

    /// Get thread identity - uses pthread_self() on Unix for fork compatibility
    #[pyfunction]
    #[must_use]
    pub fn get_ident() -> u64 {
        current_thread_id()
    }

    #[cfg(all(unix, feature = "threading"))]
    #[pyfunction]
    fn _stop_the_world_stats(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let stats = vm.state.stop_the_world.stats_snapshot();
        let d = vm.ctx.new_dict();
        d.set_item("stop_calls", vm.ctx.new_int(stats.stop_calls).into(), vm)?;
        d.set_item(
            "last_wait_ns",
            vm.ctx.new_int(stats.last_wait_ns).into(),
            vm,
        )?;
        d.set_item(
            "total_wait_ns",
            vm.ctx.new_int(stats.total_wait_ns).into(),
            vm,
        )?;
        d.set_item("max_wait_ns", vm.ctx.new_int(stats.max_wait_ns).into(), vm)?;
        d.set_item("poll_loops", vm.ctx.new_int(stats.poll_loops).into(), vm)?;
        d.set_item(
            "attached_seen",
            vm.ctx.new_int(stats.attached_seen).into(),
            vm,
        )?;
        d.set_item(
            "forced_parks",
            vm.ctx.new_int(stats.forced_parks).into(),
            vm,
        )?;
        d.set_item(
            "suspend_notifications",
            vm.ctx.new_int(stats.suspend_notifications).into(),
            vm,
        )?;
        d.set_item(
            "attach_wait_yields",
            vm.ctx.new_int(stats.attach_wait_yields).into(),
            vm,
        )?;
        d.set_item(
            "suspend_wait_yields",
            vm.ctx.new_int(stats.suspend_wait_yields).into(),
            vm,
        )?;
        d.set_item(
            "world_stopped",
            vm.ctx.new_bool(stats.world_stopped).into(),
            vm,
        )?;
        Ok(d)
    }

    #[cfg(all(unix, feature = "threading"))]
    #[pyfunction]
    fn _stop_the_world_reset_stats(vm: &VirtualMachine) {
        vm.state.stop_the_world.reset_stats();
    }

    /// Set the name of the current thread
    #[pyfunction]
    fn set_name(name: PyUtf8StrRef) {
        #[cfg(any(unix, windows))]
        host_thread::set_current_thread_name(name.as_str());
        #[cfg(not(any(unix, windows)))]
        let _ = name;
    }

    /// Get OS-level thread ID (pthread_self on Unix)
    /// This is important for fork compatibility - the ID must remain stable after fork
    #[cfg(unix)]
    fn current_thread_id() -> u64 {
        host_thread::current_thread_id()
    }

    #[cfg(not(unix))]
    fn current_thread_id() -> u64 {
        thread_to_rust_id(&thread::current())
    }

    /// Convert Rust thread to ID (used for non-unix platforms)
    #[cfg(not(unix))]
    fn thread_to_rust_id(t: &thread::Thread) -> u64 {
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
        let mut h = U64Hash { v: None };
        t.id().hash(&mut h);
        h.finish()
    }

    /// Get thread ID for a given thread handle (used by start_new_thread)
    fn thread_to_id(handle: &thread::JoinHandle<()>) -> u64 {
        #[cfg(unix)]
        {
            // On Unix, use pthread ID from the handle
            use std::os::unix::thread::JoinHandleExt;
            handle.as_pthread_t() as _
        }
        #[cfg(not(unix))]
        {
            thread_to_rust_id(handle.thread())
        }
    }

    #[pyfunction]
    const fn allocate_lock() -> Lock {
        Lock { mu: RawMutex::INIT }
    }

    #[pyfunction]
    fn start_new_thread(mut f_args: FuncArgs, vm: &VirtualMachine) -> PyResult<u64> {
        if !f_args.kwargs.is_empty() {
            return Err(vm.new_type_error("start_new_thread() takes no keyword arguments"));
        }
        let given = f_args.args.len();
        if given < 2 {
            return Err(vm.new_type_error(format!(
                "start_new_thread expected at least 2 arguments, got {given}"
            )));
        }
        if given > 3 {
            return Err(vm.new_type_error(format!(
                "start_new_thread expected at most 3 arguments, got {given}"
            )));
        }

        let func_obj = f_args.take_positional().unwrap();
        let args_obj = f_args.take_positional().unwrap();
        let kwargs_obj = f_args.take_positional();

        if func_obj.to_callable().is_none() {
            return Err(vm.new_type_error("first arg must be callable"));
        }
        if !args_obj.fast_isinstance(vm.ctx.types.tuple_type) {
            return Err(vm.new_type_error("2nd arg must be a tuple"));
        }
        if kwargs_obj
            .as_ref()
            .is_some_and(|obj| !obj.fast_isinstance(vm.ctx.types.dict_type))
        {
            return Err(vm.new_type_error("optional 3rd arg must be a dictionary"));
        }

        let func: ArgCallable = func_obj.clone().try_into_value(vm)?;
        let args: PyTupleRef = args_obj.clone().try_into_value(vm)?;
        let kwargs: Option<PyDictRef> = kwargs_obj.map(|obj| obj.try_into_value(vm)).transpose()?;

        vm.sys_module.get_attr("audit", vm)?.call(
            (
                "_thread.start_new_thread",
                func_obj,
                args_obj,
                kwargs
                    .as_ref()
                    .map_or_else(|| vm.ctx.none(), |k| k.clone().into()),
            ),
            vm,
        )?;

        if vm
            .state
            .finalizing
            .load(core::sync::atomic::Ordering::Acquire)
        {
            return Err(vm.new_exception_msg(
                vm.ctx.exceptions.python_finalization_error.to_owned(),
                "can't create new thread at interpreter shutdown"
                    .to_owned()
                    .into(),
            ));
        }

        let args = FuncArgs::new(
            args.to_vec(),
            kwargs
                .map_or_else(Default::default, |k| k.to_attributes(vm))
                .into_iter()
                .map(|(k, v)| (k.as_str().to_owned(), v))
                .collect::<KwArgs>(),
        );
        let thread_builder = apply_thread_stack_size(thread::Builder::new(), vm);
        thread_builder
            .spawn(
                vm.new_thread()
                    .make_spawn_func(move |vm| run_thread(func, args, vm)),
            )
            .map(|handle| thread_to_id(&handle))
            .map_err(|_err| vm.new_runtime_error("can't start new thread"))
    }

    fn run_thread(func: ArgCallable, args: FuncArgs, vm: &VirtualMachine) {
        // Increment thread count when thread actually starts executing
        vm.state.thread_count.fetch_add(1);

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
        // Clean up thread-local storage while VM context is still active
        // This ensures __del__ methods are called properly
        cleanup_thread_local_data();
        // Clean up frame tracking
        crate::vm::thread::cleanup_current_thread_frames(vm);
        vm.state.thread_count.fetch_sub(1);
    }

    fn apply_thread_stack_size(
        thread_builder: thread::Builder,
        vm: &VirtualMachine,
    ) -> thread::Builder {
        let configured = vm.state.stacksize.load();
        if configured != 0 {
            thread_builder.stack_size(configured)
        } else {
            thread_builder
        }
    }

    /// Clean up thread-local data for the current thread.
    /// This triggers __del__ on objects stored in thread-local variables.
    fn cleanup_thread_local_data() {
        // Take all guards - this will trigger LocalGuard::drop for each,
        // which removes the thread's dict from each Local instance
        LOCAL_GUARDS.with(|guards| {
            guards.borrow_mut().clear();
        });
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "host_env"))]
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

    #[pyfunction]
    fn daemon_threads_allowed() -> bool {
        // RustPython always allows daemon threads
        true
    }

    // Registry for non-daemon threads that need to be joined at shutdown
    pub(crate) type ShutdownEntry = (
        Weak<parking_lot::Mutex<ThreadHandleInner>>,
        Weak<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    );

    #[pyfunction]
    fn _shutdown(vm: &VirtualMachine) {
        // Wait for all non-daemon threads to finish
        let current_ident = get_ident();

        loop {
            // Find a thread that's not finished and not the current thread
            let handle_to_join = {
                let mut handles = vm.state.shutdown_handles.lock();
                // Clean up finished entries
                handles.retain(|(inner_weak, _): &ShutdownEntry| {
                    inner_weak.upgrade().is_some_and(|inner| {
                        let guard = inner.lock();
                        guard.state != ThreadHandleState::Done && guard.ident != current_ident
                    })
                });

                // Find first unfinished handle
                handles
                    .iter()
                    .find_map(|(inner_weak, done_event_weak): &ShutdownEntry| {
                        let inner = inner_weak.upgrade()?;
                        let done_event = done_event_weak.upgrade()?;
                        let guard = inner.lock();
                        if guard.state != ThreadHandleState::Done && guard.ident != current_ident {
                            Some((inner.clone(), done_event))
                        } else {
                            None
                        }
                    })
            };

            match handle_to_join {
                Some((inner, done_event)) => {
                    if let Err(exc) = ThreadHandle::join_internal(&inner, &done_event, None, vm) {
                        vm.run_unraisable(
                            exc,
                            Some(
                                "Exception ignored while joining a thread in _thread._shutdown()"
                                    .to_owned(),
                            ),
                            vm.ctx.none(),
                        );
                        return;
                    }
                }
                None => break, // No more threads to wait on
            }
        }
    }

    /// Add a non-daemon thread handle to the shutdown registry
    fn add_to_shutdown_handles(
        vm: &VirtualMachine,
        inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
        done_event: &Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    ) {
        let mut handles = vm.state.shutdown_handles.lock();
        handles.push((Arc::downgrade(inner), Arc::downgrade(done_event)));
    }

    fn remove_from_shutdown_handles(
        vm: &VirtualMachine,
        inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
        done_event: &Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    ) {
        let mut handles = vm.state.shutdown_handles.lock();
        handles.retain(|(inner_weak, done_event_weak): &ShutdownEntry| {
            let Some(registered_inner) = inner_weak.upgrade() else {
                return false;
            };
            let Some(registered_done_event) = done_event_weak.upgrade() else {
                return false;
            };
            !(Arc::ptr_eq(&registered_inner, inner)
                && Arc::ptr_eq(&registered_done_event, done_event))
        });
    }

    #[pyfunction]
    fn _make_thread_handle(ident: u64, vm: &VirtualMachine) -> PyRef<ThreadHandle> {
        let handle = ThreadHandle::new(vm);
        {
            let mut inner = handle.inner.lock();
            inner.ident = ident;
            inner.state = ThreadHandleState::Running;
        }
        handle.into_ref(&vm.ctx)
    }

    #[pyfunction]
    fn _get_main_thread_ident(vm: &VirtualMachine) -> u64 {
        vm.state.main_thread_ident.load()
    }

    #[pyfunction]
    fn _is_main_interpreter() -> bool {
        // RustPython only has one interpreter
        true
    }

    /// Initialize the main thread ident. Should be called once at interpreter startup.
    pub(crate) fn init_main_thread_ident(vm: &VirtualMachine) {
        let ident = get_ident();
        vm.state.main_thread_ident.store(ident);
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
            vm.new_type_error("_thread._excepthook argument type must be _ExceptHookArgs")
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
                .map(|s| s.as_wtf8().to_owned())
        } else {
            None
        };
        let name = thread_name.unwrap_or_else(|| Wtf8Buf::from(format!("{}", get_ident())));

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

    // Thread-local storage for cleanup guards
    // When a thread terminates, the guard is dropped, which triggers cleanup
    thread_local! {
        static LOCAL_GUARDS: RefCell<Vec<LocalGuard>> = const { RefCell::new(Vec::new()) };
    }

    // Guard that removes thread-local data when dropped
    struct LocalGuard {
        local: Weak<LocalData>,
        thread_id: u64,
    }

    impl Drop for LocalGuard {
        fn drop(&mut self) {
            if let Some(local_data) = self.local.upgrade() {
                // Remove from map while holding the lock, but drop the value
                // outside the lock to prevent deadlock if __del__ accesses _local
                let removed = local_data.data.lock().remove(&self.thread_id);
                drop(removed);
            }
        }
    }

    // Shared data structure for Local
    struct LocalData {
        data: parking_lot::Mutex<std::collections::HashMap<u64, PyDictRef>>,
    }

    impl fmt::Debug for LocalData {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("LocalData").finish_non_exhaustive()
        }
    }

    #[pyattr]
    #[pyclass(module = "_thread", name = "_local")]
    #[derive(Debug, PyPayload)]
    struct Local {
        inner: Arc<LocalData>,
    }

    #[pyclass(with(GetAttr, SetAttr), flags(BASETYPE))]
    impl Local {
        fn l_dict(&self, vm: &VirtualMachine) -> PyDictRef {
            let thread_id = current_thread_id();

            // Fast path: check if dict exists under lock
            if let Some(dict) = self.inner.data.lock().get(&thread_id).cloned() {
                return dict;
            }

            // Slow path: allocate dict outside lock to reduce lock hold time
            let new_dict = vm.ctx.new_dict();

            // Insert with double-check to handle races
            let mut data = self.inner.data.lock();
            use std::collections::hash_map::Entry;
            let (dict, need_guard) = match data.entry(thread_id) {
                Entry::Occupied(e) => (e.get().clone(), false),
                Entry::Vacant(e) => {
                    e.insert(new_dict.clone());
                    (new_dict, true)
                }
            };
            drop(data); // Release lock before TLS access

            // Register cleanup guard only if we inserted a new entry
            if need_guard {
                let guard = LocalGuard {
                    local: Arc::downgrade(&self.inner),
                    thread_id,
                };
                LOCAL_GUARDS.with(|guards| {
                    guards.borrow_mut().push(guard);
                });
            }

            dict
        }

        #[pygetset(name = "__dict__")]
        fn dict(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyDictRef {
            zelf.l_dict(vm)
        }

        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Self {
                inner: Arc::new(LocalData {
                    data: parking_lot::Mutex::new(std::collections::HashMap::new()),
                }),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    impl GetAttr for Local {
        fn getattro(zelf: &Py<Self>, attr: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            let l_dict = zelf.l_dict(vm);
            if attr.as_bytes() == b"__dict__" {
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
            if attr.as_bytes() == b"__dict__" {
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

    // Registry of all ThreadHandles for fork cleanup
    // Stores weak references so handles can be garbage collected normally
    pub(crate) type HandleEntry = (
        Weak<parking_lot::Mutex<ThreadHandleInner>>,
        Weak<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    );

    // Re-export type from vm::thread for PyGlobalState
    pub(crate) use crate::vm::thread::CurrentFrameSlot;

    /// Get all threads' current (top) frames. Used by sys._current_frames().
    pub(crate) fn get_all_current_frames(vm: &VirtualMachine) -> Vec<(u64, FrameRef)> {
        let registry = vm.state.thread_frames.lock();
        registry
            .iter()
            .filter_map(|(id, slot)| {
                let frames = slot.frames.lock();
                // SAFETY: the owning thread can't pop while we hold the Mutex,
                // so the FramePtr is valid for the duration of the lock.
                frames
                    .last()
                    .map(|fp| (*id, unsafe { fp.as_ref() }.to_owned()))
            })
            .collect()
    }

    /// Called after fork() in child process to mark all other threads as done.
    /// This prevents join() from hanging on threads that don't exist in the child.
    ///
    /// Precondition: `reinit_locks_after_fork()` has already been called, so all
    /// parking_lot-based locks in VmState are in unlocked state.
    #[cfg(unix)]
    pub(crate) fn after_fork_child(vm: &VirtualMachine) {
        let current_ident = get_ident();

        // Update main thread ident - after fork, the current thread becomes the main thread
        vm.state.main_thread_ident.store(current_ident);

        // Reinitialize frame slot for current thread.
        // Locks are already reinit'd, so lock() is safe.
        crate::vm::thread::reinit_frame_slot_after_fork(vm);

        // Clean up thread handles. All VmState locks were reinit'd to unlocked,
        // so lock() won't deadlock. Per-thread Arc<Mutex<ThreadHandleInner>>
        // locks are also reinit'd below before use.
        {
            let mut handles = vm.state.thread_handles.lock();
            handles.retain(|(inner_weak, done_event_weak): &HandleEntry| {
                let Some(inner) = inner_weak.upgrade() else {
                    return false;
                };
                let Some(done_event) = done_event_weak.upgrade() else {
                    return false;
                };

                // Reinit this per-handle lock in case a dead thread held it
                reinit_parking_lot_mutex(&inner);
                let mut inner_guard = inner.lock();

                if inner_guard.ident == current_ident {
                    return true;
                }
                if inner_guard.state == ThreadHandleState::NotStarted {
                    return true;
                }

                inner_guard.state = ThreadHandleState::Done;
                inner_guard.join_handle = None;
                drop(inner_guard);

                // Reinit and set the done event
                let (lock, cvar) = &*done_event;
                reinit_parking_lot_mutex(lock);
                *lock.lock() = true;
                cvar.notify_all();

                true
            });
        }

        // Clean up shutdown_handles.
        {
            let mut handles = vm.state.shutdown_handles.lock();
            handles.retain(|(inner_weak, done_event_weak): &ShutdownEntry| {
                let Some(inner) = inner_weak.upgrade() else {
                    return false;
                };
                let Some(done_event) = done_event_weak.upgrade() else {
                    return false;
                };

                reinit_parking_lot_mutex(&inner);
                let mut inner_guard = inner.lock();

                if inner_guard.ident == current_ident {
                    return true;
                }
                if inner_guard.state == ThreadHandleState::NotStarted {
                    return true;
                }

                inner_guard.state = ThreadHandleState::Done;
                drop(inner_guard);

                let (lock, cvar) = &*done_event;
                reinit_parking_lot_mutex(lock);
                *lock.lock() = true;
                cvar.notify_all();

                false
            });
        }
    }

    /// Reset a parking_lot::Mutex to unlocked state after fork.
    #[cfg(unix)]
    fn reinit_parking_lot_mutex<T: ?Sized>(mutex: &parking_lot::Mutex<T>) {
        unsafe { rustpython_common::lock::zero_reinit_after_fork(mutex.raw()) };
    }

    // Thread handle state enum
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ThreadHandleState {
        NotStarted,
        Starting,
        Running,
        Done,
    }

    // Internal shared state for thread handle
    pub struct ThreadHandleInner {
        pub state: ThreadHandleState,
        pub ident: u64,
        pub join_handle: Option<thread::JoinHandle<()>>,
        pub joining: bool, // True if a thread is currently joining
        pub joined: bool,  // Track if join has completed
    }

    impl fmt::Debug for ThreadHandleInner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("ThreadHandleInner")
                .field("state", &self.state)
                .field("ident", &self.ident)
                .field("join_handle", &self.join_handle.is_some())
                .field("joining", &self.joining)
                .field("joined", &self.joined)
                .finish()
        }
    }

    /// _ThreadHandle - handle for joinable threads
    #[pyattr]
    #[pyclass(module = "_thread", name = "_ThreadHandle")]
    #[derive(Debug, PyPayload)]
    struct ThreadHandle {
        inner: Arc<parking_lot::Mutex<ThreadHandleInner>>,
        // Event to signal thread completion (for timed join support)
        done_event: Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    }

    #[pyclass(with(Representable))]
    impl ThreadHandle {
        fn new(vm: &VirtualMachine) -> Self {
            let inner = Arc::new(parking_lot::Mutex::new(ThreadHandleInner {
                state: ThreadHandleState::NotStarted,
                ident: 0,
                join_handle: None,
                joining: false,
                joined: false,
            }));
            let done_event =
                Arc::new((parking_lot::Mutex::new(false), parking_lot::Condvar::new()));

            // Register in global registry for fork cleanup
            vm.state
                .thread_handles
                .lock()
                .push((Arc::downgrade(&inner), Arc::downgrade(&done_event)));

            Self { inner, done_event }
        }

        fn join_internal(
            inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
            done_event: &Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
            timeout_duration: Option<Duration>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Self::check_started(inner, vm)?;

            let deadline =
                timeout_duration.and_then(|timeout| std::time::Instant::now().checked_add(timeout));

            // Wait for thread completion using Condvar (supports timeout)
            // Loop to handle spurious wakeups
            let (lock, cvar) = &**done_event;
            let mut done = lock.lock();

            // ThreadHandle_join semantics: self-join/finalizing checks
            // apply only while target thread has not reported it is exiting yet.
            if !*done {
                let inner_guard = inner.lock();
                let current_ident = get_ident();
                if inner_guard.ident == current_ident
                    && inner_guard.state == ThreadHandleState::Running
                {
                    return Err(vm.new_runtime_error("Cannot join current thread"));
                }
                if vm
                    .state
                    .finalizing
                    .load(core::sync::atomic::Ordering::Acquire)
                {
                    return Err(vm.new_exception_msg(
                        vm.ctx.exceptions.python_finalization_error.to_owned(),
                        "cannot join thread at interpreter shutdown"
                            .to_owned()
                            .into(),
                    ));
                }
            }

            while !*done {
                if let Some(timeout) = timeout_duration {
                    let remaining = deadline.map_or(timeout, |deadline| {
                        deadline.saturating_duration_since(std::time::Instant::now())
                    });
                    if remaining.is_zero() {
                        return Ok(());
                    }
                    let result = vm.allow_threads(|| cvar.wait_for(&mut done, remaining));
                    if result.timed_out() && !*done {
                        // Timeout occurred and done is still false
                        return Ok(());
                    }
                } else {
                    // Infinite wait
                    vm.allow_threads(|| cvar.wait(&mut done));
                }
            }
            drop(done);

            // Thread is done, now perform cleanup
            let join_handle = {
                let mut inner_guard = inner.lock();

                // If already joined, return immediately (idempotent)
                if inner_guard.joined {
                    return Ok(());
                }

                // If another thread is already joining, wait for them to finish
                if inner_guard.joining {
                    drop(inner_guard);
                    // Wait on done_event
                    let (lock, cvar) = &**done_event;
                    let mut done = lock.lock();
                    while !*done {
                        vm.allow_threads(|| cvar.wait(&mut done));
                    }
                    return Ok(());
                }

                // Mark that we're joining
                inner_guard.joining = true;

                // Take the join handle if available
                inner_guard.join_handle.take()
            };

            // Perform the actual join outside the lock
            if let Some(handle) = join_handle {
                // Ignore the result - panics in spawned threads are already handled
                let _ = vm.allow_threads(|| handle.join());
            }

            // Mark as joined and clear joining flag
            {
                let mut inner_guard = inner.lock();
                inner_guard.joined = true;
                inner_guard.joining = false;
            }

            Ok(())
        }

        fn check_started(
            inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let state = inner.lock().state;
            if matches!(
                state,
                ThreadHandleState::NotStarted | ThreadHandleState::Starting
            ) {
                return Err(vm.new_runtime_error("thread not started"));
            }
            Ok(())
        }

        fn set_done_internal(
            inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
            done_event: &Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Self::check_started(inner, vm)?;
            {
                let mut inner_guard = inner.lock();
                inner_guard.state = ThreadHandleState::Done;
                // _set_done() detach path. Dropping the JoinHandle
                // detaches the underlying Rust thread.
                inner_guard.join_handle = None;
                inner_guard.joining = false;
                inner_guard.joined = true;
            }
            remove_from_shutdown_handles(vm, inner, done_event);

            let (lock, cvar) = &**done_event;
            *lock.lock() = true;
            cvar.notify_all();
            Ok(())
        }

        fn parse_join_timeout(
            timeout_obj: Option<crate::PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<Duration>> {
            const JOIN_TIMEOUT_MAX_SECONDS: i64 = TIMEOUT_MAX_IN_MICROSECONDS / 1_000_000;
            let Some(timeout_obj) = timeout_obj else {
                return Ok(None);
            };

            if let Some(t) = timeout_obj.try_index_opt(vm) {
                let t: i64 = t?.try_to_primitive(vm).map_err(|_| {
                    vm.new_overflow_error("timestamp too large to convert to C PyTime_t")
                })?;
                if !(-JOIN_TIMEOUT_MAX_SECONDS..=JOIN_TIMEOUT_MAX_SECONDS).contains(&t) {
                    return Err(
                        vm.new_overflow_error("timestamp too large to convert to C PyTime_t")
                    );
                }
                if t < 0 {
                    return Ok(None);
                }
                return Ok(Some(Duration::from_secs(t as u64)));
            }

            if let Some(t) = timeout_obj.try_float_opt(vm) {
                let t = t?.to_f64();
                if t.is_nan() {
                    return Err(vm.new_value_error("Invalid value NaN (not a number)"));
                }
                if !t.is_finite() || !(-TIMEOUT_MAX..=TIMEOUT_MAX).contains(&t) {
                    return Err(vm.new_overflow_error("timestamp out of range for platform time_t"));
                }
                if t < 0.0 {
                    return Ok(None);
                }
                return Ok(Some(Duration::from_secs_f64(t)));
            }

            Err(vm.new_type_error(format!(
                "'{}' object cannot be interpreted as an integer or float",
                timeout_obj.class().name()
            )))
        }

        #[pygetset]
        fn ident(&self) -> u64 {
            self.inner.lock().ident
        }

        #[pymethod]
        fn is_done(&self, f_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
            if !f_args.kwargs.is_empty() {
                return Err(vm.new_type_error("_ThreadHandle.is_done() takes no keyword arguments"));
            }
            let given = f_args.args.len();
            if given != 0 {
                return Err(vm.new_type_error(format!(
                    "_ThreadHandle.is_done() takes no arguments ({given} given)"
                )));
            }

            // If completion was observed, perform one-time join cleanup
            // before returning True.
            let done = {
                let (lock, _) = &*self.done_event;
                *lock.lock()
            };
            if !done {
                return Ok(false);
            }
            Self::join_internal(&self.inner, &self.done_event, Some(Duration::ZERO), vm)?;
            Ok(true)
        }

        #[pymethod]
        fn _set_done(&self, f_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            if !f_args.kwargs.is_empty() {
                return Err(
                    vm.new_type_error("_ThreadHandle._set_done() takes no keyword arguments")
                );
            }
            let given = f_args.args.len();
            if given != 0 {
                return Err(vm.new_type_error(format!(
                    "_ThreadHandle._set_done() takes no arguments ({given} given)"
                )));
            }

            Self::set_done_internal(&self.inner, &self.done_event, vm)
        }

        #[pymethod]
        fn join(&self, mut f_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            if !f_args.kwargs.is_empty() {
                return Err(vm.new_type_error("_ThreadHandle.join() takes no keyword arguments"));
            }
            let given = f_args.args.len();
            if given > 1 {
                return Err(
                    vm.new_type_error(format!("join() takes at most 1 argument ({given} given)"))
                );
            }
            let timeout = f_args.take_positional().filter(|obj| !vm.is_none(obj));
            let timeout_duration = Self::parse_join_timeout(timeout, vm)?;
            Self::join_internal(&self.inner, &self.done_event, timeout_duration, vm)
        }

        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            ThreadHandle::new(vm)
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    impl Representable for ThreadHandle {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let ident = zelf.inner.lock().ident;
            Ok(format!(
                "<{} object: ident={ident}>",
                zelf.class().slot_name()
            ))
        }
    }

    #[pyfunction]
    fn start_joinable_thread(
        mut f_args: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<ThreadHandle>> {
        let given = f_args.args.len() + f_args.kwargs.len();
        if given > 3 {
            return Err(vm.new_type_error(format!(
                "start_joinable_thread() takes at most 3 arguments ({given} given)"
            )));
        }

        let function_pos = f_args.take_positional();
        let function_kw = f_args.take_keyword("function");
        if function_pos.is_some() && function_kw.is_some() {
            return Err(vm.new_type_error(
                "argument for start_joinable_thread() given by name ('function') and position (1)",
            ));
        }
        let Some(function_obj) = function_pos.or(function_kw) else {
            return Err(vm.new_type_error(
                "start_joinable_thread() missing required argument 'function' (pos 1)",
            ));
        };

        let handle_pos = f_args.take_positional();
        let handle_kw = f_args.take_keyword("handle");
        if handle_pos.is_some() && handle_kw.is_some() {
            return Err(vm.new_type_error(
                "argument for start_joinable_thread() given by name ('handle') and position (2)",
            ));
        }
        let handle_obj = handle_pos.or(handle_kw);

        let daemon_pos = f_args.take_positional();
        let daemon_kw = f_args.take_keyword("daemon");
        if daemon_pos.is_some() && daemon_kw.is_some() {
            return Err(vm.new_type_error(
                "argument for start_joinable_thread() given by name ('daemon') and position (3)",
            ));
        }
        let daemon = daemon_pos
            .or(daemon_kw)
            .map_or(Ok(true), |obj| obj.try_to_bool(vm))?;

        // Match CPython parser precedence:
        // - required positional/keyword argument errors are raised before
        //   unknown keyword errors when `function` is missing.
        if let Some(unexpected) = f_args.kwargs.keys().next() {
            let suggestion = ["function", "handle", "daemon"]
                .iter()
                .filter_map(|candidate| {
                    let max_distance = (unexpected.len() + candidate.len() + 3) * MOVE_COST / 6;
                    let distance = levenshtein_distance(
                        unexpected.as_bytes(),
                        candidate.as_bytes(),
                        max_distance,
                    );
                    (distance <= max_distance).then_some((distance, *candidate))
                })
                .min_by_key(|(distance, _)| *distance)
                .map(|(_, candidate)| candidate);
            let msg = if let Some(suggestion) = suggestion {
                format!(
                    "start_joinable_thread() got an unexpected keyword argument '{unexpected}'. Did you mean '{suggestion}'?"
                )
            } else {
                format!("start_joinable_thread() got an unexpected keyword argument '{unexpected}'")
            };
            return Err(vm.new_type_error(msg));
        }

        if function_obj.to_callable().is_none() {
            return Err(vm.new_type_error("thread function must be callable"));
        }
        let function: ArgCallable = function_obj.clone().try_into_value(vm)?;

        let thread_handle_type = ThreadHandle::class(&vm.ctx);
        let handle = if let Some(handle_obj) = handle_obj {
            if vm.is_none(&handle_obj) {
                None
            } else if !handle_obj.class().is(thread_handle_type) {
                return Err(vm.new_type_error("'handle' must be a _ThreadHandle"));
            } else {
                Some(
                    handle_obj
                        .downcast::<ThreadHandle>()
                        .map_err(|_| vm.new_type_error("'handle' must be a _ThreadHandle"))?,
                )
            }
        } else {
            None
        };

        vm.sys_module.get_attr("audit", vm)?.call(
            (
                "_thread.start_joinable_thread",
                function_obj,
                daemon,
                handle
                    .as_ref()
                    .map_or_else(|| vm.ctx.none(), |h| h.clone().into()),
            ),
            vm,
        )?;

        if vm
            .state
            .finalizing
            .load(core::sync::atomic::Ordering::Acquire)
        {
            return Err(vm.new_exception_msg(
                vm.ctx.exceptions.python_finalization_error.to_owned(),
                "can't create new thread at interpreter shutdown"
                    .to_owned()
                    .into(),
            ));
        }

        let handle = match handle {
            Some(h) => h,
            None => ThreadHandle::new(vm).into_ref(&vm.ctx),
        };

        // Must only start once (ThreadHandle_start).
        {
            let mut inner = handle.inner.lock();
            if inner.state != ThreadHandleState::NotStarted {
                return Err(vm.new_runtime_error("thread already started"));
            }
            inner.state = ThreadHandleState::Starting;
            inner.ident = 0;
            inner.join_handle = None;
            inner.joining = false;
            inner.joined = false;
        }
        // Starting a handle always resets the completion event.
        {
            let (done_lock, _) = &*handle.done_event;
            *done_lock.lock() = false;
        }

        // Add non-daemon threads to shutdown registry so _shutdown() will wait for them
        if !daemon {
            add_to_shutdown_handles(vm, &handle.inner, &handle.done_event);
        }

        let func = function;
        let handle_clone = handle.clone();
        let inner_clone = handle.inner.clone();
        let done_event_clone = handle.done_event.clone();
        // Use std::sync (pthread-based) instead of parking_lot for these
        // events so they remain fork-safe without the parking_lot_core patch.
        let started_event = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let started_event_clone = Arc::clone(&started_event);
        let handle_ready_event =
            Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let handle_ready_event_clone = Arc::clone(&handle_ready_event);

        let thread_builder = apply_thread_stack_size(thread::Builder::new(), vm);

        let join_handle = thread_builder
            .spawn(vm.new_thread().make_spawn_func(move |vm| {
                // Publish ident for the parent starter thread.
                {
                    inner_clone.lock().ident = get_ident();
                }
                {
                    let (started_lock, started_cvar) = &*started_event_clone;
                    *started_lock.lock().unwrap() = true;
                    started_cvar.notify_all();
                }
                // Don't execute the target function until parent marks the
                // handle as running.
                {
                    let (ready_lock, ready_cvar) = &*handle_ready_event_clone;
                    let mut ready = ready_lock.lock().unwrap();
                    while !*ready {
                        // Short timeout so we stay responsive to STW requests.
                        let (guard, _) = ready_cvar
                            .wait_timeout(ready, core::time::Duration::from_millis(1))
                            .unwrap();
                        ready = guard;
                    }
                }

                // Ensure cleanup happens even if the function panics
                let inner_for_cleanup = inner_clone.clone();
                let done_event_for_cleanup = done_event_clone.clone();
                let vm_state = vm.state.clone();
                scopeguard::defer! {
                    // Mark as done
                    inner_for_cleanup.lock().state = ThreadHandleState::Done;

                    // Handle sentinels
                    for lock in SENTINELS.take() {
                        if lock.mu.is_locked() {
                            unsafe { lock.mu.unlock() };
                        }
                    }

                    // Clean up thread-local data while VM context is still active
                    cleanup_thread_local_data();

                    // Clean up frame tracking
                    crate::vm::thread::cleanup_current_thread_frames(vm);

                    vm_state.thread_count.fetch_sub(1);

                    // The runtime no longer needs to wait for this thread.
                    remove_from_shutdown_handles(vm, &inner_for_cleanup, &done_event_for_cleanup);

                    // Signal waiting threads that this thread is done
                    // This must be LAST to ensure all cleanup is complete before join() returns
                    {
                        let (lock, cvar) = &*done_event_for_cleanup;
                        *lock.lock() = true;
                        cvar.notify_all();
                    }
                }

                // Increment thread count when thread actually starts executing
                vm_state.thread_count.fetch_add(1);

                // Run the function
                match func.invoke((), vm) {
                    Ok(_) => {}
                    Err(e) if e.fast_isinstance(vm.ctx.exceptions.system_exit) => {}
                    Err(exc) => {
                        vm.run_unraisable(
                            exc,
                            Some("Exception ignored in thread started by".to_owned()),
                            func.into(),
                        );
                    }
                }
            }))
            .map_err(|_err| {
                // force_done + remove_from_shutdown_handles on start failure.
                {
                    let mut inner = handle.inner.lock();
                    inner.state = ThreadHandleState::Done;
                    inner.join_handle = None;
                    inner.joining = false;
                    inner.joined = true;
                }
                {
                    let (done_lock, done_cvar) = &*handle.done_event;
                    *done_lock.lock() = true;
                    done_cvar.notify_all();
                }
                if !daemon {
                    remove_from_shutdown_handles(vm, &handle.inner, &handle.done_event);
                }
                vm.new_runtime_error("can't start new thread")
            })?;

        // Wait until the new thread has reported its ident.
        {
            let (started_lock, started_cvar) = &*started_event;
            let mut started = started_lock.lock().unwrap();
            while !*started {
                let (guard, _) = started_cvar
                    .wait_timeout(started, core::time::Duration::from_millis(1))
                    .unwrap();
                started = guard;
            }
        }

        // Mark the handle running in the parent thread (like CPython's
        // ThreadHandle_start sets THREAD_HANDLE_RUNNING after spawn succeeds).
        {
            let mut inner = handle.inner.lock();
            inner.join_handle = Some(join_handle);
            inner.state = ThreadHandleState::Running;
        }

        // Unblock the started thread once handle state is fully published.
        {
            let (ready_lock, ready_cvar) = &*handle_ready_event;
            *ready_lock.lock().unwrap() = true;
            ready_cvar.notify_all();
        }

        Ok(handle_clone)
    }
}
