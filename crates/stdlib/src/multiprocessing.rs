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

#[cfg(unix)]
#[pymodule]
mod _multiprocessing {
    use crate::vm::{
        Context, FromArgs, Py, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyType, PyTypeRef},
        function::{FuncArgs, OptionalArg},
        types::Constructor,
    };
    use libc::sem_t;
    use nix::errno::Errno;
    use std::{
        ffi::CString,
        sync::atomic::{AtomicU64, AtomicUsize, Ordering},
        time::Duration,
    };
    unsafe extern "C" {
        fn sem_getvalue(sem: *mut sem_t, sval: *mut libc::c_int) -> libc::c_int;
        fn sem_timedwait(sem: *mut sem_t, abs_timeout: *const libc::timespec) -> libc::c_int;
    }

    const RECURSIVE_MUTEX_KIND: i32 = 0;
    const SEMAPHORE_KIND: i32 = 1;
    const SEM_VALUE_MAX_CONST: i32 = 32_767;

    #[derive(FromArgs)]
    struct SemLockArgs {
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

    #[derive(FromArgs)]
    struct AcquireArgs {
        #[pyarg(any, default = true)]
        blocking: bool,
        #[pyarg(any, default = OptionalArg::Missing)]
        timeout: OptionalArg<Option<f64>>,
    }

    #[pyattr]
    #[pyclass(name = "SemLock", module = "_multiprocessing")]
    #[derive(Debug, PyPayload)]
    struct SemLock {
        handle: SemHandle,
        kind: i32,
        maxvalue: i32,
        name: Option<String>,
        owner: AtomicU64,
        count: AtomicUsize,
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
            let raw = unsafe {
                libc::sem_open(cname.as_ptr(), libc::O_CREAT | libc::O_EXCL, 0o600, value)
            };
            if raw == libc::SEM_FAILED {
                let err = Errno::last();
                return Err(os_error(vm, err, None));
            }
            if unlink {
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
                return Err(os_error(vm, err, None));
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
            if !self.raw.is_null() {
                unsafe {
                    libc::sem_close(self.raw);
                }
            }
        }
    }

    #[pyclass(with(Constructor))]
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

        #[pymethod]
        fn acquire(&self, args: AcquireArgs, vm: &VirtualMachine) -> PyResult<bool> {
            let blocking = args.blocking;
            let timeout = match args.timeout {
                OptionalArg::Missing => None,
                OptionalArg::Present(v) => v,
            };
            if !blocking && timeout.is_some() {
                return Err(vm.new_value_error(
                    "can't specify a timeout for a non-blocking call".to_owned(),
                ));
            }

            let tid = current_thread_id();
            if self.kind == RECURSIVE_MUTEX_KIND && self.owner.load(Ordering::Acquire) == tid {
                self.count.fetch_add(1, Ordering::Relaxed);
                return Ok(true);
            }

            let acquired = if !blocking {
                self.try_wait(vm)?
            } else if let Some(secs) = timeout {
                let duration = duration_from_secs(vm, secs)?;
                self.wait_timeout(duration, vm)?
            } else {
                self.wait(vm)?;
                true
            };

            if acquired {
                if self.owner.load(Ordering::Acquire) == tid {
                    self.count.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.owner.store(tid, Ordering::Release);
                    self.count.store(1, Ordering::Release);
                }
            }
            Ok(acquired)
        }

        #[pymethod]
        fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
            let tid = current_thread_id();
            if self.kind == RECURSIVE_MUTEX_KIND && self.owner.load(Ordering::Acquire) != tid {
                return Err(vm.new_value_error("cannot release un-acquired lock".to_owned()));
            }

            let owner_tid = self.owner.load(Ordering::Acquire);
            if owner_tid == tid {
                let current = self.count.load(Ordering::Acquire);
                if current == 0 {
                    return Err(vm.new_value_error("cannot release un-acquired lock".to_owned()));
                }
                if self.kind == RECURSIVE_MUTEX_KIND && current > 1 {
                    self.count.store(current - 1, Ordering::Release);
                    return Ok(());
                }
                let new_val = current.saturating_sub(1);
                self.count.store(new_val, Ordering::Release);
                if new_val == 0 {
                    self.owner.store(0, Ordering::Release);
                }
            } else if self.kind != RECURSIVE_MUTEX_KIND {
                // releasing semaphore or non-recursive lock from another thread;
                // drop ownership information.
                self.owner.store(0, Ordering::Release);
                self.count.store(0, Ordering::Release);
            }

            let res = unsafe { libc::sem_post(self.handle.as_ptr()) };
            if res == -1 {
                let err = Errno::last();
                return Err(os_error(vm, err, None));
            }
            Ok(())
        }

        #[pymethod(name = "__enter__")]
        fn enter(&self, vm: &VirtualMachine) -> PyResult<bool> {
            self.acquire(
                AcquireArgs {
                    blocking: true,
                    timeout: OptionalArg::Missing,
                },
                vm,
            )
        }

        #[pymethod]
        fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.release(vm)
        }

        #[pyclassmethod]
        #[pymethod(name = "_rebuild")]
        fn rebuild(
            cls: PyTypeRef,
            _handle: isize,
            kind: i32,
            maxvalue: i32,
            name: Option<String>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let Some(name) = name else {
                return Err(vm.new_value_error("semaphore name missing".to_owned()));
            };
            let handle = SemHandle::open_existing(&name, vm)?;
            let zelf = SemLock {
                handle,
                kind,
                maxvalue,
                name: Some(name),
                owner: AtomicU64::new(0),
                count: AtomicUsize::new(0),
            };
            zelf.into_ref_with_type(vm, cls).map(Into::into)
        }

        #[pymethod]
        fn _after_fork(&self, _vm: &VirtualMachine) -> PyResult<()> {
            self.owner.store(0, Ordering::Release);
            self.count.store(0, Ordering::Release);
            Ok(())
        }

        #[pymethod]
        fn _get_value(&self, vm: &VirtualMachine) -> PyResult<i32> {
            let mut value = 0;
            let res = unsafe { libc::sem_getvalue(self.handle.as_ptr(), &mut value) };
            if res == -1 {
                let err = Errno::last();
                return Err(os_error(vm, err, None));
            }
            Ok(value)
        }

        #[pymethod]
        fn _is_zero(&self, vm: &VirtualMachine) -> PyResult<bool> {
            Ok(self._get_value(vm)? == 0)
        }

        #[pymethod]
        fn _is_mine(&self) -> bool {
            self.owner.load(Ordering::Acquire) == current_thread_id()
        }

        #[pymethod]
        fn _count(&self) -> usize {
            if self._is_mine() {
                self.count.load(Ordering::Acquire)
            } else {
                0
            }
        }

        #[extend_class]
        fn extend_class(ctx: &Context, class: &Py<PyType>) {
            class.set_attr(
                ctx.intern_str("RECURSIVE_MUTEX"),
                ctx.new_int(RECURSIVE_MUTEX_KIND).into(),
            );
            class.set_attr(
                ctx.intern_str("SEMAPHORE"),
                ctx.new_int(SEMAPHORE_KIND).into(),
            );
            class.set_attr(
                ctx.intern_str("SEM_VALUE_MAX"),
                ctx.new_int(SEM_VALUE_MAX_CONST).into(),
            );
        }

        fn wait(&self, vm: &VirtualMachine) -> PyResult<()> {
            loop {
                let res = unsafe { libc::sem_wait(self.handle.as_ptr()) };
                if res == 0 {
                    return Ok(());
                }
                let err = Errno::last();
                if err == Errno::EINTR {
                    continue;
                }
                return Err(os_error(vm, err, None));
            }
        }

        fn try_wait(&self, vm: &VirtualMachine) -> PyResult<bool> {
            let res = unsafe { libc::sem_trywait(self.handle.as_ptr()) };
            if res == 0 {
                return Ok(true);
            }
            let err = Errno::last();
            if err == Errno::EAGAIN {
                return Ok(false);
            }
            Err(os_error(vm, err, None))
        }

        fn wait_timeout(&self, duration: Duration, vm: &VirtualMachine) -> PyResult<bool> {
            let mut ts = current_timespec(vm)?;
            let nsec_total = ts.tv_nsec as i64 + i64::from(duration.subsec_nanos());
            ts.tv_sec = ts
                .tv_sec
                .saturating_add(duration.as_secs() as libc::time_t + nsec_total / 1_000_000_000);
            ts.tv_nsec = (nsec_total % 1_000_000_000) as _;
            loop {
                let res = unsafe { libc::sem_timedwait(self.handle.as_ptr(), &ts) };
                if res == 0 {
                    return Ok(true);
                }
                let err = Errno::last();
                match err {
                    Errno::EINTR => continue,
                    Errno::ETIMEDOUT => return Ok(false),
                    other => return Err(os_error(vm, other, None)),
                }
            }
        }
    }

    impl Constructor for SemLock {
        type Args = SemLockArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            if args.value < 0 || args.value > args.maxvalue {
                return Err(vm.new_value_error("semaphore or lock value out of range".to_owned()));
            }
            let value = u32::try_from(args.value).map_err(|_| {
                vm.new_value_error("semaphore or lock value out of range".to_owned())
            })?;
            let (handle, name) = SemHandle::create(&args.name, value, args.unlink, vm)?;
            Ok(SemLock {
                handle,
                kind: args.kind,
                maxvalue: args.maxvalue,
                name,
                owner: AtomicU64::new(0),
                count: AtomicUsize::new(0),
            })
        }
    }

    #[pyfunction]
    fn sem_unlink(name: String, vm: &VirtualMachine) -> PyResult<()> {
        let cname = semaphore_name(vm, &name)?;
        let res = unsafe { libc::sem_unlink(cname.as_ptr()) };
        if res == -1 {
            let err = Errno::last();
            return Err(os_error(vm, err, None));
        }
        Ok(())
    }

    fn current_timespec(vm: &VirtualMachine) -> PyResult<libc::timespec> {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let res = unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
        if res != 0 {
            return Err(vm.new_os_error("Failed to get clock time"));
        }
        Ok(ts)
    }

    fn duration_from_secs(vm: &VirtualMachine, secs: f64) -> PyResult<Duration> {
        if !secs.is_finite() {
            return Err(vm.new_overflow_error("timestamp too large".to_owned()));
        }
        if secs < 0.0 {
            return Err(vm.new_value_error("timeout value out of range".to_owned()));
        }
        Ok(Duration::from_secs_f64(secs))
    }

    fn semaphore_name(vm: &VirtualMachine, name: &str) -> PyResult<CString> {
        let mut full = String::with_capacity(name.len() + 1);
        if !name.starts_with('/') {
            full.push('/');
        }
        full.push_str(name);
        CString::new(full).map_err(|_| vm.new_value_error("embedded null character".to_owned()))
    }

    fn os_error(vm: &VirtualMachine, err: Errno, msg: Option<String>) -> PyBaseExceptionRef {
        let exc_type = match err {
            Errno::EEXIST => vm.ctx.exceptions.file_exists_error.to_owned(),
            Errno::ENOENT => vm.ctx.exceptions.file_not_found_error.to_owned(),
            _ => vm.ctx.exceptions.os_error.to_owned(),
        };
        let text = msg.unwrap_or_else(|| err.desc().to_owned());
        vm.new_os_subtype_error(exc_type, Some(err as i32), text)
            .upcast()
    }

    fn current_thread_id() -> u64 {
        unsafe { libc::pthread_self() as u64 }
    }
}

#[cfg(all(not(unix), not(windows)))]
#[pymodule]
mod _multiprocessing {}
