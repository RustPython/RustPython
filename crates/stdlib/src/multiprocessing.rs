pub(crate) use _multiprocessing::module_def;

#[cfg(windows)]
#[pymodule]
mod _multiprocessing {
    use crate::vm::{
        Context, FromArgs, Py, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyDict, PyType, PyTypeRef},
        convert::ToPyException,
        function::{ArgBytesLike, FuncArgs, KwArgs},
        types::Constructor,
    };
    use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};
    use rustpython_host_env::multiprocessing as host_multiprocessing;

    // These match the values in Lib/multiprocessing/synchronize.py
    const RECURSIVE_MUTEX: i32 = 0;
    const SEMAPHORE: i32 = 1;

    macro_rules! ismine {
        ($self:expr) => {
            $self.count.load(Ordering::Acquire) > 0
                && $self.last_tid.load(Ordering::Acquire)
                    == host_multiprocessing::current_thread_id()
        };
    }

    #[derive(FromArgs)]
    struct SemLockNewArgs {
        #[pyarg(positional)]
        kind: i32,
        #[pyarg(positional)]
        value: i32,
        #[pyarg(positional)]
        maxvalue: i32,
        #[pyarg(positional)]
        name: String,
        #[pyarg(positional)]
        unlink: bool,
    }

    #[pyattr]
    #[pyclass(name = "SemLock", module = "_multiprocessing")]
    #[derive(Debug, PyPayload)]
    struct SemLock {
        handle: SemHandle,
        kind: i32,
        maxvalue: i32,
        name: Option<String>,
        last_tid: AtomicU32,
        count: AtomicI32,
    }

    type SemHandle = host_multiprocessing::SemHandle;

    #[pyclass(with(Constructor), flags(BASETYPE))]
    impl SemLock {
        #[pygetset]
        fn handle(&self) -> isize {
            self.handle.as_raw() as isize
        }

        #[pygetset]
        fn kind(&self) -> i32 {
            self.kind
        }

        #[pygetset]
        fn maxvalue(&self) -> i32 {
            self.maxvalue
        }

        #[pygetset]
        fn name(&self) -> Option<String> {
            self.name.clone()
        }

        #[pymethod]
        fn acquire(&self, args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
            let blocking: bool = args
                .kwargs
                .get("block")
                .or_else(|| args.args.first())
                .map(|o| o.clone().try_to_bool(vm))
                .transpose()?
                .unwrap_or(true);

            let timeout_obj = args
                .kwargs
                .get("timeout")
                .or_else(|| args.args.get(1))
                .cloned();

            // Calculate timeout in milliseconds
            let full_msecs: u32 = if !blocking {
                0
            } else if timeout_obj.as_ref().is_none_or(|o| vm.is_none(o)) {
                host_multiprocessing::INFINITE_TIMEOUT
            } else {
                let timeout: f64 = timeout_obj.unwrap().try_float(vm)?.to_f64();
                let timeout = timeout * 1000.0; // convert to ms
                if timeout < 0.0 {
                    0
                } else if timeout >= 0.5 * host_multiprocessing::INFINITE_TIMEOUT as f64 {
                    return Err(vm.new_overflow_error("timeout is too large"));
                } else {
                    (timeout + 0.5) as u32
                }
            };

            // Check whether we already own the lock
            if self.kind == RECURSIVE_MUTEX && ismine!(self) {
                self.count.fetch_add(1, Ordering::Release);
                return Ok(true);
            }

            // Check whether we can acquire without blocking
            match host_multiprocessing::wait_for_single_object(self.handle.as_raw(), 0) {
                x if x == host_multiprocessing::wait_object_0() => {
                    self.last_tid
                        .store(host_multiprocessing::current_thread_id(), Ordering::Release);
                    self.count.fetch_add(1, Ordering::Release);
                    return Ok(true);
                }
                x if x == host_multiprocessing::wait_failed() => return Err(vm.new_last_os_error()),
                _ => {}
            }

            // Poll with signal checking (CPython uses WaitForMultipleObjectsEx
            // with sigint_event; we poll since RustPython has no sigint event)
            let poll_ms: u32 = 100;
            let mut elapsed: u32 = 0;
            loop {
                let wait_ms = if full_msecs == host_multiprocessing::INFINITE_TIMEOUT {
                    poll_ms
                } else {
                    let remaining = full_msecs.saturating_sub(elapsed);
                    if remaining == 0 {
                        return Ok(false);
                    }
                    remaining.min(poll_ms)
                };

                let handle = self.handle.as_raw();
                let res = vm.allow_threads(|| {
                    host_multiprocessing::wait_for_single_object(handle, wait_ms)
                });

                match res {
                    x if x == host_multiprocessing::wait_object_0() => {
                        self.last_tid
                            .store(host_multiprocessing::current_thread_id(), Ordering::Release);
                        self.count.fetch_add(1, Ordering::Release);
                        return Ok(true);
                    }
                    x if x == host_multiprocessing::wait_timeout() => {
                        vm.check_signals()?;
                        if full_msecs != host_multiprocessing::INFINITE_TIMEOUT {
                            elapsed = elapsed.saturating_add(wait_ms);
                        }
                    }
                    x if x == host_multiprocessing::wait_failed() => {
                        return Err(vm.new_last_os_error());
                    }
                    _ => {
                        return Err(vm.new_runtime_error(format!(
                            "WaitForSingleObject() gave unrecognized value {res}"
                        )));
                    }
                }
            }
        }

        #[pymethod]
        fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.kind == RECURSIVE_MUTEX {
                if !ismine!(self) {
                    return Err(vm.new_exception_msg(
                        vm.ctx.exceptions.assertion_error.to_owned(),
                        "attempt to release recursive lock not owned by thread".into(),
                    ));
                }
                if self.count.load(Ordering::Acquire) > 1 {
                    self.count.fetch_sub(1, Ordering::Release);
                    return Ok(());
                }
            }

            if let Err(err) = host_multiprocessing::release_semaphore(self.handle.as_raw()) {
                if host_multiprocessing::is_too_many_posts(err) {
                    return Err(vm.new_value_error("semaphore or lock released too many times"));
                }
                return Err(vm.new_last_os_error());
            }

            self.count.fetch_sub(1, Ordering::Release);
            Ok(())
        }

        #[pymethod(name = "__enter__")]
        fn enter(&self, vm: &VirtualMachine) -> PyResult<bool> {
            self.acquire(
                FuncArgs::new::<Vec<_>, KwArgs>(
                    vec![vm.ctx.new_bool(true).into()],
                    KwArgs::default(),
                ),
                vm,
            )
        }

        #[pymethod]
        fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.release(vm)
        }

        #[pyclassmethod(name = "_rebuild")]
        fn rebuild(
            cls: PyTypeRef,
            handle: isize,
            kind: i32,
            maxvalue: i32,
            name: Option<String>,
            vm: &VirtualMachine,
        ) -> PyResult {
            // On Windows, _rebuild receives the handle directly (no sem_open)
            let zelf = SemLock {
                handle: SemHandle::from_raw(handle as host_multiprocessing::RawHandle),
                kind,
                maxvalue,
                name,
                last_tid: AtomicU32::new(0),
                count: AtomicI32::new(0),
            };
            zelf.into_ref_with_type(vm, cls).map(Into::into)
        }

        #[pymethod]
        fn _after_fork(&self) {
            self.count.store(0, Ordering::Release);
            self.last_tid.store(0, Ordering::Release);
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot pickle 'SemLock' object"))
        }

        #[pymethod]
        fn _count(&self) -> i32 {
            self.count.load(Ordering::Acquire)
        }

        #[pymethod]
        fn _is_mine(&self) -> bool {
            ismine!(self)
        }

        #[pymethod]
        fn _get_value(&self, vm: &VirtualMachine) -> PyResult<i32> {
            host_multiprocessing::get_semaphore_value(self.handle.as_raw())
                .map_err(|_| vm.new_last_os_error())
        }

        #[pymethod]
        fn _is_zero(&self, vm: &VirtualMachine) -> PyResult<bool> {
            let val = host_multiprocessing::get_semaphore_value(self.handle.as_raw())
                .map_err(|_| vm.new_last_os_error())?;
            Ok(val == 0)
        }

        #[extend_class]
        fn extend_class(ctx: &Context, class: &Py<PyType>) {
            class.set_attr(
                ctx.intern_str("RECURSIVE_MUTEX"),
                ctx.new_int(RECURSIVE_MUTEX).into(),
            );
            class.set_attr(ctx.intern_str("SEMAPHORE"), ctx.new_int(SEMAPHORE).into());
            class.set_attr(
                ctx.intern_str("SEM_VALUE_MAX"),
                ctx.new_int(i32::MAX).into(),
            );
        }
    }

    impl Constructor for SemLock {
        type Args = SemLockNewArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            if args.kind != RECURSIVE_MUTEX && args.kind != SEMAPHORE {
                return Err(vm.new_value_error("unrecognized kind"));
            }
            if args.maxvalue <= 0 {
                return Err(vm.new_value_error("maxvalue must be positive"));
            }
            if args.value < 0 || args.value > args.maxvalue {
                return Err(vm.new_value_error("invalid value"));
            }

            let handle =
                SemHandle::create(args.value, args.maxvalue).map_err(|e| e.to_pyexception(vm))?;
            let name = if args.unlink { None } else { Some(args.name) };

            Ok(SemLock {
                handle,
                kind: args.kind,
                maxvalue: args.maxvalue,
                name,
                last_tid: AtomicU32::new(0),
                count: AtomicI32::new(0),
            })
        }
    }

    // On Windows, sem_unlink is a no-op
    #[pyfunction]
    fn sem_unlink(_name: String) {}

    #[pyattr]
    fn flags(vm: &VirtualMachine) -> PyRef<PyDict> {
        // On Windows, no HAVE_SEM_OPEN / HAVE_SEM_TIMEDWAIT / HAVE_BROKEN_SEM_GETVALUE
        vm.ctx.new_dict()
    }

    #[pyfunction]
    fn closesocket(socket: usize, vm: &VirtualMachine) -> PyResult<()> {
        host_multiprocessing::close_socket(socket as host_multiprocessing::RawSocket)
            .map_err(|_| vm.new_last_os_error())
    }

    #[pyfunction]
    fn recv(socket: usize, size: usize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        host_multiprocessing::recv_socket(socket as host_multiprocessing::RawSocket, size)
            .map_err(|_| vm.new_last_os_error())
    }

    #[pyfunction]
    fn send(socket: usize, buf: ArgBytesLike, vm: &VirtualMachine) -> PyResult<libc::c_int> {
        buf.with_ref(|b| {
            host_multiprocessing::send_socket(socket as host_multiprocessing::RawSocket, b)
        })
        .map_err(|_| vm.new_last_os_error())
    }
}

// Unix platforms (Linux, macOS, etc.)
// macOS has broken sem_timedwait/sem_getvalue - we use polled fallback
#[cfg(unix)]
#[pymodule]
mod _multiprocessing {
    use crate::vm::{
        Context, FromArgs, Py, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyDict, PyType, PyTypeRef},
        function::{FuncArgs, KwArgs},
        types::Constructor,
    };
    use core::sync::atomic::{AtomicI32, AtomicU64, Ordering};
    #[cfg(target_vendor = "apple")]
    use libc::sem_t;
    use rustpython_host_env::multiprocessing::{
        self as host_multiprocessing, SemError, TryAcquireStatus, WaitStatus,
    };

    /// Error type for sem_timedwait operations
    #[cfg(target_vendor = "apple")]
    enum SemWaitError {
        Timeout,
        SignalException(PyBaseExceptionRef),
        OsError(SemError),
    }

    /// macOS fallback for sem_timedwait using select + sem_trywait polling
    /// Matches sem_timedwait_save in semaphore.c
    #[cfg(target_vendor = "apple")]
    fn sem_timedwait_polled(
        sem: *mut sem_t,
        deadline: &libc::timespec,
        vm: &VirtualMachine,
    ) -> Result<(), SemWaitError> {
        let mut delay: u64 = 0;

        loop {
            match vm.allow_threads(|| {
                host_multiprocessing::sem_timedwait_poll_step(sem, deadline, delay)
            }) {
                Ok(host_multiprocessing::PollWaitStep::Acquired) => return Ok(()),
                Ok(host_multiprocessing::PollWaitStep::Timeout) => {
                    return Err(SemWaitError::Timeout);
                }
                Ok(host_multiprocessing::PollWaitStep::Continue(next_delay)) => {
                    delay = next_delay;
                }
                Err(err) => return Err(SemWaitError::OsError(err)),
            }

            // check for signals - preserve the exception (e.g., KeyboardInterrupt)
            if let Err(exc) = vm.check_signals() {
                return Err(SemWaitError::SignalException(exc));
            }
        }
    }

    // These match the values in Lib/multiprocessing/synchronize.py
    const RECURSIVE_MUTEX: i32 = 0;
    const SEMAPHORE: i32 = 1;

    // #define ISMINE(o) (o->count > 0 && PyThread_get_thread_ident() == o->last_tid)
    macro_rules! ismine {
        ($self:expr) => {
            $self.count.load(Ordering::Acquire) > 0
                && $self.last_tid.load(Ordering::Acquire)
                    == host_multiprocessing::current_thread_id()
        };
    }

    #[derive(FromArgs)]
    struct SemLockNewArgs {
        #[pyarg(positional)]
        kind: i32,
        #[pyarg(positional)]
        value: i32,
        #[pyarg(positional)]
        maxvalue: i32,
        #[pyarg(positional)]
        name: String,
        #[pyarg(positional)]
        unlink: bool,
    }

    #[pyattr]
    #[pyclass(name = "SemLock", module = "_multiprocessing")]
    #[derive(Debug, PyPayload)]
    struct SemLock {
        handle: SemHandle,
        kind: i32,
        maxvalue: i32,
        name: Option<String>,
        last_tid: AtomicU64, // unsigned long
        count: AtomicI32,    // int
    }

    type SemHandle = host_multiprocessing::SemHandle;

    #[pyclass(with(Constructor), flags(BASETYPE))]
    impl SemLock {
        #[pygetset]
        fn handle(&self) -> isize {
            self.handle.as_ptr() as isize
        }

        #[pygetset]
        fn kind(&self) -> i32 {
            self.kind
        }

        #[pygetset]
        fn maxvalue(&self) -> i32 {
            self.maxvalue
        }

        #[pygetset]
        fn name(&self) -> Option<String> {
            self.name.clone()
        }

        /// Acquire the semaphore/lock.
        // _multiprocessing_SemLock_acquire_impl
        #[pymethod]
        fn acquire(&self, args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
            // block=True, timeout=None

            let blocking: bool = args
                .kwargs
                .get("block")
                .or_else(|| args.args.first())
                .map(|o| o.clone().try_to_bool(vm))
                .transpose()?
                .unwrap_or(true);

            let timeout_obj = args
                .kwargs
                .get("timeout")
                .or_else(|| args.args.get(1))
                .cloned();

            if self.kind == RECURSIVE_MUTEX && ismine!(self) {
                self.count.fetch_add(1, Ordering::Release);
                return Ok(true);
            }

            // timeout_obj != Py_None
            let use_deadline = timeout_obj.as_ref().is_some_and(|o| !vm.is_none(o));

            let deadline = if use_deadline {
                let timeout_obj = timeout_obj.unwrap();
                // This accepts both int and float, converting to f64
                let timeout: f64 = timeout_obj.try_float(vm)?.to_f64();
                Some(
                    host_multiprocessing::deadline_from_timeout(timeout)
                        .map_err(|_| vm.new_os_error("gettimeofday failed".to_string()))?,
                )
            } else {
                None
            };

            // Check whether we can acquire without releasing the GIL and blocking
            let try_status = loop {
                match host_multiprocessing::sem_trywait_status(self.handle.as_ptr()) {
                    TryAcquireStatus::Interrupted => {
                        vm.check_signals()?;
                    }
                    status => break status,
                }
            };

            // if (res < 0 && errno == EAGAIN && blocking)
            if matches!(try_status, TryAcquireStatus::WouldBlock) && blocking {
                // Couldn't acquire immediately, need to block.
                //
                // Save errno inside the allow_threads closure, before
                // attach_thread() runs — matches CPython which saves
                // `err = errno` before Py_END_ALLOW_THREADS.

                #[cfg(not(target_vendor = "apple"))]
                {
                    loop {
                        let sem_ptr = self.handle.as_ptr();
                        // Py_BEGIN_ALLOW_THREADS / Py_END_ALLOW_THREADS
                        match vm.allow_threads(|| {
                            host_multiprocessing::sem_wait_status(sem_ptr, deadline.as_ref())
                        }) {
                            WaitStatus::Acquired => break,
                            WaitStatus::Interrupted => {
                                vm.check_signals()?;
                                continue;
                            }
                            WaitStatus::TimedOut => return Ok(false),
                            WaitStatus::Error(err) => return Err(os_error(vm, err)),
                        }
                    }
                }
                #[cfg(target_vendor = "apple")]
                {
                    // macOS: use polled fallback since sem_timedwait is not available
                    if let Some(ref dl) = deadline {
                        match sem_timedwait_polled(self.handle.as_ptr(), dl, vm) {
                            Ok(()) => {}
                            Err(SemWaitError::Timeout) => {
                                return Ok(false);
                            }
                            Err(SemWaitError::SignalException(exc)) => {
                                return Err(exc);
                            }
                            Err(SemWaitError::OsError(e)) => {
                                return Err(os_error(vm, e));
                            }
                        }
                    } else {
                        // No timeout: use sem_wait (available on macOS)
                        loop {
                            let sem_ptr = self.handle.as_ptr();
                            match vm.allow_threads(|| {
                                host_multiprocessing::sem_wait_status(sem_ptr, None)
                            }) {
                                WaitStatus::Acquired => break,
                                WaitStatus::Interrupted => {
                                    vm.check_signals()?;
                                    continue;
                                }
                                WaitStatus::TimedOut => return Ok(false),
                                WaitStatus::Error(err) => return Err(os_error(vm, err)),
                            }
                        }
                    }
                }
            } else if !matches!(try_status, TryAcquireStatus::Acquired) {
                // Non-blocking path failed, or blocking=false
                match try_status {
                    TryAcquireStatus::WouldBlock => return Ok(false),
                    TryAcquireStatus::Interrupted => return vm.check_signals().map(|_| false),
                    TryAcquireStatus::Error(err) => return Err(os_error(vm, err)),
                    TryAcquireStatus::Acquired => unreachable!(),
                }
            }

            self.count.fetch_add(1, Ordering::Release);
            self.last_tid
                .store(host_multiprocessing::current_thread_id(), Ordering::Release);

            Ok(true)
        }

        /// Release the semaphore/lock.
        // _multiprocessing_SemLock_release_impl
        #[pymethod]
        fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.kind == RECURSIVE_MUTEX {
                // if (!ISMINE(self))
                if !ismine!(self) {
                    return Err(vm.new_exception_msg(
                        vm.ctx.exceptions.assertion_error.to_owned(),
                        "attempt to release recursive lock not owned by thread".into(),
                    ));
                }
                // if (self->count > 1) { --self->count; Py_RETURN_NONE; }
                if self.count.load(Ordering::Acquire) > 1 {
                    self.count.fetch_sub(1, Ordering::Release);
                    return Ok(());
                }
                // assert(self->count == 1);
            } else {
                // SEMAPHORE case: check value before releasing
                #[cfg(not(target_vendor = "apple"))]
                {
                    // Linux: use sem_getvalue
                    let sval =
                        unsafe { host_multiprocessing::get_semaphore_value(self.handle.as_ptr()) }
                            .map_err(|err| os_error(vm, err))?;
                    if sval >= self.maxvalue {
                        return Err(vm.new_value_error("semaphore or lock released too many times"));
                    }
                }
                #[cfg(target_vendor = "apple")]
                {
                    // macOS: HAVE_BROKEN_SEM_GETVALUE
                    // We will only check properly the maxvalue == 1 case
                    if self.maxvalue == 1 {
                        // make sure that already locked
                        match host_multiprocessing::sem_trywait_status(self.handle.as_ptr()) {
                            TryAcquireStatus::WouldBlock => {}
                            TryAcquireStatus::Acquired => {
                                if let Err(err) =
                                    host_multiprocessing::sem_post(self.handle.as_ptr())
                                {
                                    return Err(os_error(vm, err));
                                }
                                return Err(
                                    vm.new_value_error("semaphore or lock released too many times")
                                );
                            }
                            TryAcquireStatus::Interrupted => {
                                return Err(os_error(vm, SemError::Interrupted));
                            }
                            TryAcquireStatus::Error(err) => return Err(os_error(vm, err)),
                        }
                    }
                }
            }

            if let Err(err) = host_multiprocessing::sem_post(self.handle.as_ptr()) {
                return Err(os_error(vm, err));
            }

            self.count.fetch_sub(1, Ordering::Release);
            Ok(())
        }

        /// Enter the semaphore/lock (context manager).
        // _multiprocessing_SemLock___enter___impl
        #[pymethod(name = "__enter__")]
        fn enter(&self, vm: &VirtualMachine) -> PyResult<bool> {
            // return _multiprocessing_SemLock_acquire_impl(self, 1, Py_None);
            self.acquire(
                FuncArgs::new::<Vec<_>, KwArgs>(
                    vec![vm.ctx.new_bool(true).into()],
                    KwArgs::default(),
                ),
                vm,
            )
        }

        /// Exit the semaphore/lock (context manager).
        // _multiprocessing_SemLock___exit___impl
        #[pymethod]
        fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.release(vm)
        }

        /// Rebuild a SemLock from pickled state.
        // _multiprocessing_SemLock__rebuild_impl
        #[pyclassmethod(name = "_rebuild")]
        fn rebuild(
            cls: PyTypeRef,
            _handle: isize,
            kind: i32,
            maxvalue: i32,
            name: Option<String>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let Some(ref name_str) = name else {
                return Err(vm.new_value_error("cannot rebuild SemLock without name"));
            };
            let handle = SemHandle::open_existing(name_str).map_err(|err| os_error(vm, err))?;
            // return newsemlockobject(type, handle, kind, maxvalue, name_copy);
            let zelf = SemLock {
                handle,
                kind,
                maxvalue,
                name,
                last_tid: AtomicU64::new(0),
                count: AtomicI32::new(0),
            };
            zelf.into_ref_with_type(vm, cls).map(Into::into)
        }

        /// Rezero the net acquisition count after fork().
        // _multiprocessing_SemLock__after_fork_impl
        #[pymethod]
        fn _after_fork(&self) {
            self.count.store(0, Ordering::Release);
            // Also reset last_tid for safety
            self.last_tid.store(0, Ordering::Release);
        }

        /// SemLock objects cannot be pickled directly.
        /// Use multiprocessing.synchronize.SemLock wrapper which handles pickling.
        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot pickle 'SemLock' object"))
        }

        /// Num of `acquire()`s minus num of `release()`s for this process.
        // _multiprocessing_SemLock__count_impl
        #[pymethod]
        fn _count(&self) -> i32 {
            self.count.load(Ordering::Acquire)
        }

        /// Whether the lock is owned by this thread.
        // _multiprocessing_SemLock__is_mine_impl
        #[pymethod]
        fn _is_mine(&self) -> bool {
            ismine!(self)
        }

        /// Get the value of the semaphore.
        // _multiprocessing_SemLock__get_value_impl
        #[pymethod]
        fn _get_value(&self, vm: &VirtualMachine) -> PyResult<i32> {
            #[cfg(not(target_vendor = "apple"))]
            {
                // Linux: use sem_getvalue
                unsafe { host_multiprocessing::get_semaphore_value(self.handle.as_ptr()) }
                    .map_err(|err| os_error(vm, err))
            }
            #[cfg(target_vendor = "apple")]
            {
                // macOS: HAVE_BROKEN_SEM_GETVALUE - raise NotImplementedError
                Err(vm.new_not_implemented_error(String::new()))
            }
        }

        /// Return whether semaphore has value zero.
        // _multiprocessing_SemLock__is_zero_impl
        #[pymethod]
        fn _is_zero(&self, vm: &VirtualMachine) -> PyResult<bool> {
            #[cfg(not(target_vendor = "apple"))]
            {
                Ok(self._get_value(vm)? == 0)
            }
            #[cfg(target_vendor = "apple")]
            {
                // macOS: HAVE_BROKEN_SEM_GETVALUE
                // Try to acquire - if EAGAIN, value is 0
                match host_multiprocessing::sem_trywait_status(self.handle.as_ptr()) {
                    TryAcquireStatus::WouldBlock => return Ok(true),
                    TryAcquireStatus::Interrupted => {
                        return Err(os_error(vm, SemError::Interrupted));
                    }
                    TryAcquireStatus::Error(err) => return Err(os_error(vm, err)),
                    TryAcquireStatus::Acquired => {}
                }
                // Successfully acquired - undo and return false
                if let Err(err) = host_multiprocessing::sem_post(self.handle.as_ptr()) {
                    return Err(os_error(vm, err));
                }
                Ok(false)
            }
        }

        #[extend_class]
        fn extend_class(ctx: &Context, class: &Py<PyType>) {
            class.set_attr(
                ctx.intern_str("RECURSIVE_MUTEX"),
                ctx.new_int(RECURSIVE_MUTEX).into(),
            );
            class.set_attr(ctx.intern_str("SEMAPHORE"), ctx.new_int(SEMAPHORE).into());
            // SEM_VALUE_MAX from system, or INT_MAX if negative
            // We use a reasonable default
            let sem_value_max = host_multiprocessing::sem_value_max();
            class.set_attr(
                ctx.intern_str("SEM_VALUE_MAX"),
                ctx.new_int(sem_value_max).into(),
            );
        }
    }

    impl Constructor for SemLock {
        type Args = SemLockNewArgs;

        // Create a new SemLock.
        // _multiprocessing_SemLock_impl
        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            if args.kind != RECURSIVE_MUTEX && args.kind != SEMAPHORE {
                return Err(vm.new_value_error("unrecognized kind"));
            }
            // Value validation
            if args.value < 0 || args.value > args.maxvalue {
                return Err(vm.new_value_error("invalid value"));
            }

            let value = args.value as u32;
            let (handle, name) =
                SemHandle::create(&args.name, value, args.unlink).map_err(|err| {
                    if err == SemError::InvalidInput && args.name.contains('\0') {
                        vm.new_value_error("embedded null character")
                    } else {
                        os_error(vm, err)
                    }
                })?;

            // return newsemlockobject(type, handle, kind, maxvalue, name_copy);
            Ok(SemLock {
                handle,
                kind: args.kind,
                maxvalue: args.maxvalue,
                name,
                last_tid: AtomicU64::new(0),
                count: AtomicI32::new(0),
            })
        }
    }

    /// Function to unlink semaphore names.
    // _PyMp_sem_unlink.
    #[pyfunction]
    fn sem_unlink(name: String, vm: &VirtualMachine) -> PyResult<()> {
        host_multiprocessing::sem_unlink(&name).map_err(|err| {
            if err == SemError::InvalidInput && name.contains('\0') {
                vm.new_value_error("embedded null character")
            } else {
                os_error(vm, err)
            }
        })
    }

    /// Module-level flags dict.
    #[pyattr]
    fn flags(vm: &VirtualMachine) -> PyRef<PyDict> {
        let flags = vm.ctx.new_dict();
        // HAVE_SEM_OPEN is always 1 on Unix (we wouldn't be here otherwise)
        flags
            .set_item("HAVE_SEM_OPEN", vm.ctx.new_int(1).into(), vm)
            .unwrap();

        #[cfg(not(target_vendor = "apple"))]
        {
            // Linux: HAVE_SEM_TIMEDWAIT is available
            flags
                .set_item("HAVE_SEM_TIMEDWAIT", vm.ctx.new_int(1).into(), vm)
                .unwrap();
        }

        #[cfg(target_vendor = "apple")]
        {
            // macOS: sem_getvalue is broken
            flags
                .set_item("HAVE_BROKEN_SEM_GETVALUE", vm.ctx.new_int(1).into(), vm)
                .unwrap();
        }

        flags
    }

    fn os_error(vm: &VirtualMachine, err: SemError) -> PyBaseExceptionRef {
        // _PyMp_SetError maps to PyErr_SetFromErrno
        let exc_type = match err {
            SemError::AlreadyExists => vm.ctx.exceptions.file_exists_error.to_owned(),
            SemError::NotFound => vm.ctx.exceptions.file_not_found_error.to_owned(),
            _ => vm.ctx.exceptions.os_error.to_owned(),
        };
        vm.new_os_subtype_error(exc_type, Some(err.raw_os_error()), err.description())
            .upcast()
    }
}

#[cfg(all(not(unix), not(windows)))]
#[pymodule]
mod _multiprocessing {}
