pub(crate) use _multiprocessing::make_module;

#[cfg(windows)]
#[pymodule]
mod _multiprocessing {
    use crate::vm::{PyResult, VirtualMachine, function::ArgBytesLike};
    use windows_sys::Win32::Networking::WinSock::{self, SOCKET};

    #[pyfunction]
    fn closesocket(socket: usize, vm: &VirtualMachine) -> PyResult<()> {
        let res = unsafe { WinSock::closesocket(socket as SOCKET) };
        if res != 0 {
            Err(vm.new_last_os_error())
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn recv(socket: usize, size: usize, vm: &VirtualMachine) -> PyResult<libc::c_int> {
        let mut buf = vec![0; size];
        let n_read =
            unsafe { WinSock::recv(socket as SOCKET, buf.as_mut_ptr() as *mut _, size as i32, 0) };
        if n_read < 0 {
            Err(vm.new_last_os_error())
        } else {
            Ok(n_read)
        }
    }

    #[pyfunction]
    fn send(socket: usize, buf: ArgBytesLike, vm: &VirtualMachine) -> PyResult<libc::c_int> {
        let ret = buf.with_ref(|b| unsafe {
            WinSock::send(socket as SOCKET, b.as_ptr() as *const _, b.len() as i32, 0)
        });
        if ret < 0 {
            Err(vm.new_last_os_error())
        } else {
            Ok(ret)
        }
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
    use libc::sem_t;
    use nix::errno::Errno;
    use std::{
        ffi::CString,
        sync::atomic::{AtomicI32, AtomicU64, Ordering},
    };

    /// Error type for sem_timedwait operations
    #[cfg(target_vendor = "apple")]
    enum SemWaitError {
        Timeout,
        SignalException(PyBaseExceptionRef),
        OsError(Errno),
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
            // poll: try to acquire
            if unsafe { libc::sem_trywait(sem) } == 0 {
                return Ok(());
            }
            let err = Errno::last();
            if err != Errno::EAGAIN {
                return Err(SemWaitError::OsError(err));
            }

            // get current time
            let mut now = libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            };
            if unsafe { libc::gettimeofday(&mut now, std::ptr::null_mut()) } < 0 {
                return Err(SemWaitError::OsError(Errno::last()));
            }

            // check for timeout
            let deadline_usec = deadline.tv_sec * 1_000_000 + deadline.tv_nsec / 1000;
            #[allow(clippy::unnecessary_cast)]
            let now_usec = now.tv_sec as i64 * 1_000_000 + now.tv_usec as i64;

            if now_usec >= deadline_usec {
                return Err(SemWaitError::Timeout);
            }

            // calculate how much time is left
            let difference = (deadline_usec - now_usec) as u64;

            // check delay not too long -- maximum is 20 msecs
            delay += 1000;
            if delay > 20000 {
                delay = 20000;
            }
            if delay > difference {
                delay = difference;
            }

            // sleep using select
            let mut tv_delay = libc::timeval {
                tv_sec: (delay / 1_000_000) as _,
                tv_usec: (delay % 1_000_000) as _,
            };
            unsafe {
                libc::select(
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut tv_delay,
                )
            };

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
                && $self.last_tid.load(Ordering::Acquire) == current_thread_id()
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

    #[derive(Debug)]
    struct SemHandle {
        raw: *mut sem_t,
    }

    unsafe impl Send for SemHandle {}
    unsafe impl Sync for SemHandle {}

    impl SemHandle {
        fn create(
            name: &str,
            value: u32,
            unlink: bool,
            vm: &VirtualMachine,
        ) -> PyResult<(Self, Option<String>)> {
            let cname = semaphore_name(vm, name)?;
            // SEM_CREATE(name, val, max) sem_open(name, O_CREAT | O_EXCL, 0600, val)
            let raw = unsafe {
                libc::sem_open(cname.as_ptr(), libc::O_CREAT | libc::O_EXCL, 0o600, value)
            };
            if raw == libc::SEM_FAILED {
                let err = Errno::last();
                return Err(os_error(vm, err));
            }
            if unlink {
                // SEM_UNLINK(name) sem_unlink(name)
                unsafe {
                    libc::sem_unlink(cname.as_ptr());
                }
                Ok((SemHandle { raw }, None))
            } else {
                Ok((SemHandle { raw }, Some(name.to_owned())))
            }
        }

        fn open_existing(name: &str, vm: &VirtualMachine) -> PyResult<Self> {
            let cname = semaphore_name(vm, name)?;
            let raw = unsafe { libc::sem_open(cname.as_ptr(), 0) };
            if raw == libc::SEM_FAILED {
                let err = Errno::last();
                return Err(os_error(vm, err));
            }
            Ok(SemHandle { raw })
        }

        #[inline]
        fn as_ptr(&self) -> *mut sem_t {
            self.raw
        }
    }

    impl Drop for SemHandle {
        fn drop(&mut self) {
            // Guard against default/uninitialized state.
            // Note: SEM_FAILED is (sem_t*)-1, not null, but valid handles are never null
            // and SEM_FAILED is never stored (error is returned immediately on sem_open failure).
            if !self.raw.is_null() {
                // SEM_CLOSE(sem) sem_close(sem)
                unsafe {
                    libc::sem_close(self.raw);
                }
            }
        }
    }

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
                let timeout = if timeout < 0.0 { 0.0 } else { timeout };

                let mut tv = libc::timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                };
                let res = unsafe { libc::gettimeofday(&mut tv, std::ptr::null_mut()) };
                if res < 0 {
                    return Err(vm.new_os_error("gettimeofday failed".to_string()));
                }

                // deadline calculation:
                // long sec = (long) timeout;
                // long nsec = (long) (1e9 * (timeout - sec) + 0.5);
                // deadline.tv_sec = now.tv_sec + sec;
                // deadline.tv_nsec = now.tv_usec * 1000 + nsec;
                // deadline.tv_sec += (deadline.tv_nsec / 1000000000);
                // deadline.tv_nsec %= 1000000000;
                let sec = timeout as libc::c_long;
                let nsec = (1e9 * (timeout - sec as f64) + 0.5) as libc::c_long;
                let mut deadline = libc::timespec {
                    tv_sec: tv.tv_sec + sec as libc::time_t,
                    tv_nsec: (tv.tv_usec as libc::c_long * 1000 + nsec) as _,
                };
                deadline.tv_sec += (deadline.tv_nsec / 1_000_000_000) as libc::time_t;
                deadline.tv_nsec %= 1_000_000_000;
                Some(deadline)
            } else {
                None
            };

            // Check whether we can acquire without releasing the GIL and blocking
            let mut res;
            loop {
                res = unsafe { libc::sem_trywait(self.handle.as_ptr()) };
                if res >= 0 {
                    break;
                }
                let err = Errno::last();
                if err == Errno::EINTR {
                    vm.check_signals()?;
                    continue;
                }
                break;
            }

            // if (res < 0 && errno == EAGAIN && blocking)
            if res < 0 && Errno::last() == Errno::EAGAIN && blocking {
                // Couldn't acquire immediately, need to block
                #[cfg(not(target_vendor = "apple"))]
                {
                    loop {
                        // Py_BEGIN_ALLOW_THREADS / Py_END_ALLOW_THREADS
                        // RustPython doesn't have GIL, so we just do the wait
                        if let Some(ref dl) = deadline {
                            res = unsafe { libc::sem_timedwait(self.handle.as_ptr(), dl) };
                        } else {
                            res = unsafe { libc::sem_wait(self.handle.as_ptr()) };
                        }

                        if res >= 0 {
                            break;
                        }
                        let err = Errno::last();
                        if err == Errno::EINTR {
                            vm.check_signals()?;
                            continue;
                        }
                        break;
                    }
                }
                #[cfg(target_vendor = "apple")]
                {
                    // macOS: use polled fallback since sem_timedwait is not available
                    if let Some(ref dl) = deadline {
                        match sem_timedwait_polled(self.handle.as_ptr(), dl, vm) {
                            Ok(()) => res = 0,
                            Err(SemWaitError::Timeout) => {
                                // Timeout occurred - return false directly
                                return Ok(false);
                            }
                            Err(SemWaitError::SignalException(exc)) => {
                                // Propagate the original exception (e.g., KeyboardInterrupt)
                                return Err(exc);
                            }
                            Err(SemWaitError::OsError(e)) => {
                                return Err(os_error(vm, e));
                            }
                        }
                    } else {
                        // No timeout: use sem_wait (available on macOS)
                        loop {
                            res = unsafe { libc::sem_wait(self.handle.as_ptr()) };
                            if res >= 0 {
                                break;
                            }
                            let err = Errno::last();
                            if err == Errno::EINTR {
                                vm.check_signals()?;
                                continue;
                            }
                            break;
                        }
                    }
                }
            }

            // result handling:
            if res < 0 {
                let err = Errno::last();
                match err {
                    Errno::EAGAIN | Errno::ETIMEDOUT => return Ok(false),
                    Errno::EINTR => {
                        // EINTR should be handled by the check_signals() loop above
                        // If we reach here, check signals again and propagate any exception
                        return vm.check_signals().map(|_| false);
                    }
                    _ => return Err(os_error(vm, err)),
                }
            }

            self.count.fetch_add(1, Ordering::Release);
            self.last_tid.store(current_thread_id(), Ordering::Release);

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
                        "attempt to release recursive lock not owned by thread".to_owned(),
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
                    let mut sval: libc::c_int = 0;
                    let res = unsafe { libc::sem_getvalue(self.handle.as_ptr(), &mut sval) };
                    if res < 0 {
                        return Err(os_error(vm, Errno::last()));
                    }
                    if sval >= self.maxvalue {
                        return Err(vm.new_value_error(
                            "semaphore or lock released too many times".to_owned(),
                        ));
                    }
                }
                #[cfg(target_vendor = "apple")]
                {
                    // macOS: HAVE_BROKEN_SEM_GETVALUE
                    // We will only check properly the maxvalue == 1 case
                    if self.maxvalue == 1 {
                        // make sure that already locked
                        if unsafe { libc::sem_trywait(self.handle.as_ptr()) } < 0 {
                            if Errno::last() != Errno::EAGAIN {
                                return Err(os_error(vm, Errno::last()));
                            }
                            // it is already locked as expected
                        } else {
                            // it was not locked so undo wait and raise
                            if unsafe { libc::sem_post(self.handle.as_ptr()) } < 0 {
                                return Err(os_error(vm, Errno::last()));
                            }
                            return Err(vm.new_value_error(
                                "semaphore or lock released too many times".to_owned(),
                            ));
                        }
                    }
                }
            }

            let res = unsafe { libc::sem_post(self.handle.as_ptr()) };
            if res < 0 {
                return Err(os_error(vm, Errno::last()));
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
                return Err(vm.new_value_error("cannot rebuild SemLock without name".to_owned()));
            };
            let handle = SemHandle::open_existing(name_str, vm)?;
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
            Err(vm.new_type_error("cannot pickle 'SemLock' object".to_owned()))
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
                let mut sval: libc::c_int = 0;
                let res = unsafe { libc::sem_getvalue(self.handle.as_ptr(), &mut sval) };
                if res < 0 {
                    return Err(os_error(vm, Errno::last()));
                }
                // some posix implementations use negative numbers to indicate
                // the number of waiting threads
                Ok(if sval < 0 { 0 } else { sval })
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
                if unsafe { libc::sem_trywait(self.handle.as_ptr()) } < 0 {
                    if Errno::last() == Errno::EAGAIN {
                        return Ok(true);
                    }
                    return Err(os_error(vm, Errno::last()));
                }
                // Successfully acquired - undo and return false
                if unsafe { libc::sem_post(self.handle.as_ptr()) } < 0 {
                    return Err(os_error(vm, Errno::last()));
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
            let sem_value_max: i32 = unsafe {
                let val = libc::sysconf(libc::_SC_SEM_VALUE_MAX);
                if val < 0 || val > i32::MAX as libc::c_long {
                    i32::MAX
                } else {
                    val as i32
                }
            };
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
                return Err(vm.new_value_error("unrecognized kind".to_owned()));
            }
            // Value validation
            if args.value < 0 || args.value > args.maxvalue {
                return Err(vm.new_value_error("invalid value".to_owned()));
            }

            let value = args.value as u32;
            let (handle, name) = SemHandle::create(&args.name, value, args.unlink, vm)?;

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
        let cname = semaphore_name(vm, &name)?;
        let res = unsafe { libc::sem_unlink(cname.as_ptr()) };
        if res < 0 {
            return Err(os_error(vm, Errno::last()));
        }
        Ok(())
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

    fn semaphore_name(vm: &VirtualMachine, name: &str) -> PyResult<CString> {
        // POSIX semaphore names must start with /
        let mut full = String::with_capacity(name.len() + 1);
        if !name.starts_with('/') {
            full.push('/');
        }
        full.push_str(name);
        CString::new(full).map_err(|_| vm.new_value_error("embedded null character".to_owned()))
    }

    fn os_error(vm: &VirtualMachine, err: Errno) -> PyBaseExceptionRef {
        // _PyMp_SetError maps to PyErr_SetFromErrno
        let exc_type = match err {
            Errno::EEXIST => vm.ctx.exceptions.file_exists_error.to_owned(),
            Errno::ENOENT => vm.ctx.exceptions.file_not_found_error.to_owned(),
            _ => vm.ctx.exceptions.os_error.to_owned(),
        };
        vm.new_os_subtype_error(exc_type, Some(err as i32), err.desc().to_owned())
            .upcast()
    }

    /// Get current thread identifier.
    /// PyThread_get_thread_ident on Unix (pthread_self).
    fn current_thread_id() -> u64 {
        unsafe { libc::pthread_self() as u64 }
    }
}

#[cfg(all(not(unix), not(windows)))]
#[pymodule]
mod _multiprocessing {}
