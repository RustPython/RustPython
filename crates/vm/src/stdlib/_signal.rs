// spell-checker:disable

pub(crate) use _signal::module_def;

#[pymodule]
pub(crate) mod _signal {
    #[cfg(any(unix, windows))]
    use crate::convert::{IntoPyException, TryFromBorrowedObject};
    use crate::{Py, PyObjectRef, PyResult, VirtualMachine, signal};
    #[cfg(unix)]
    use crate::{
        builtins::PyTypeRef,
        function::{ArgIntoFloat, OptionalArg},
    };
    use core::sync::atomic::{self, Ordering};
    #[cfg(any(unix, windows))]
    use rustpython_host_env::signal::{self as host_signal, sighandler_t};
    #[cfg(unix)]
    use rustpython_host_env::signal::{double_to_timeval, itimerval_to_tuple};

    #[allow(non_camel_case_types)]
    #[cfg(not(any(unix, windows)))]
    type sighandler_t = usize;

    cfg_select! {
        windows => {
            type WakeupFdRaw = libc::SOCKET;
            struct WakeupFd(WakeupFdRaw);
            const INVALID_WAKEUP: libc::SOCKET = windows_sys::Win32::Networking::WinSock::INVALID_SOCKET;
            static WAKEUP: atomic::AtomicUsize = atomic::AtomicUsize::new(INVALID_WAKEUP);
            // windows doesn't use the same fds for files and sockets like windows does, so we need
            // this to know whether to send() or write()
            static WAKEUP_IS_SOCKET: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

            impl<'a> TryFromBorrowedObject<'a> for WakeupFd {
                fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a crate::PyObject) -> PyResult<Self> {
                    use num_traits::One;

                    let fd: &crate::Py<crate::builtins::PyInt> = obj.try_to_value(vm)?;
                    match fd.try_to_primitive::<usize>(vm) {
                        Ok(fd) => Ok(WakeupFd(fd as _)),
                        Err(e) => if (-fd.as_bigint()).is_one() {
                            Ok(WakeupFd(INVALID_WAKEUP))
                        } else {
                            Err(e)
                        },
                    }
                }
            }
        }
        _ => {
            type WakeupFdRaw = i32;
            type WakeupFd = WakeupFdRaw;
            const INVALID_WAKEUP: WakeupFd = -1;
            static WAKEUP: atomic::AtomicI32 = atomic::AtomicI32::new(INVALID_WAKEUP);
        }
    }

    #[cfg(unix)]
    #[allow(unused_imports)]
    pub use libc::SIG_ERR;
    #[cfg(unix)]
    pub use nix::unistd::alarm as sig_alarm;

    #[cfg(unix)]
    #[pyattr]
    pub use libc::{SIG_DFL, SIG_IGN};

    // pthread_sigmask 'how' constants
    #[cfg(unix)]
    #[pyattr]
    use libc::{SIG_BLOCK, SIG_SETMASK, SIG_UNBLOCK};

    #[cfg(not(unix))]
    #[pyattr]
    pub const SIG_DFL: sighandler_t = 0;
    #[cfg(not(unix))]
    #[pyattr]
    pub const SIG_IGN: sighandler_t = 1;
    #[cfg(not(unix))]
    #[allow(dead_code)]
    pub const SIG_ERR: sighandler_t = -1 as _;

    #[pyattr]
    use crate::signal::NSIG;

    #[cfg(any(unix, windows))]
    #[pyattr]
    pub use libc::{SIGABRT, SIGFPE, SIGILL, SIGINT, SIGSEGV, SIGTERM};

    #[cfg(windows)]
    #[pyattr]
    const SIGBREAK: i32 = 21; // _SIGBREAK

    // Windows-specific control events for GenerateConsoleCtrlEvent
    #[cfg(windows)]
    #[pyattr]
    const CTRL_C_EVENT: u32 = 0;
    #[cfg(windows)]
    #[pyattr]
    const CTRL_BREAK_EVENT: u32 = 1;

    #[cfg(unix)]
    #[pyattr]
    use libc::{
        SIGALRM, SIGBUS, SIGCHLD, SIGCONT, SIGHUP, SIGIO, SIGKILL, SIGPIPE, SIGPROF, SIGQUIT,
        SIGSTOP, SIGSYS, SIGTRAP, SIGTSTP, SIGTTIN, SIGTTOU, SIGURG, SIGUSR1, SIGUSR2, SIGVTALRM,
        SIGWINCH, SIGXCPU, SIGXFSZ,
    };

    #[cfg(unix)]
    #[cfg(not(any(
        target_vendor = "apple",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd"
    )))]
    #[pyattr]
    use libc::{SIGPWR, SIGSTKFLT};

    // Interval timer constants
    #[cfg(all(unix, not(target_os = "android")))]
    #[pyattr]
    use libc::{ITIMER_PROF, ITIMER_REAL, ITIMER_VIRTUAL};

    #[cfg(target_os = "android")]
    #[pyattr]
    const ITIMER_REAL: libc::c_int = 0;
    #[cfg(target_os = "android")]
    #[pyattr]
    const ITIMER_VIRTUAL: libc::c_int = 1;
    #[cfg(target_os = "android")]
    #[pyattr]
    const ITIMER_PROF: libc::c_int = 2;

    #[cfg(unix)]
    #[pyattr(name = "ItimerError", once)]
    fn itimer_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "signal",
            "ItimerError",
            Some(vec![vm.ctx.exceptions.os_error.to_owned()]),
        )
    }

    #[cfg(any(unix, windows))]
    pub(super) fn init_signal_handlers(
        module: &Py<crate::builtins::PyModule>,
        vm: &VirtualMachine,
    ) {
        if vm.state.config.settings.install_signal_handlers {
            let sig_dfl = vm.new_pyobj(SIG_DFL as u8);
            let sig_ign = vm.new_pyobj(SIG_IGN as u8);

            for signum in 1..NSIG {
                let Some(handler) = (unsafe { host_signal::probe_handler(signum as i32) }) else {
                    continue;
                };
                let py_handler = if handler == SIG_DFL {
                    Some(sig_dfl.clone())
                } else if handler == SIG_IGN {
                    Some(sig_ign.clone())
                } else {
                    None
                };
                vm.signal_handlers
                    .get_or_init(signal::new_signal_handlers)
                    .borrow_mut()[signum] = py_handler;
            }

            let int_handler = module
                .get_attr("default_int_handler", vm)
                .expect("_signal does not have this attr?");
            signal(libc::SIGINT, int_handler, vm).expect("Failed to set sigint handler");
        }
    }

    #[cfg(not(any(unix, windows)))]
    #[pyfunction]
    pub fn signal(
        _signalnum: i32,
        _handler: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        Err(vm.new_not_implemented_error("signal is not implemented on this platform"))
    }

    #[cfg(any(unix, windows))]
    #[pyfunction]
    pub fn signal(
        signalnum: i32,
        handler: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        signal::assert_in_range(signalnum, vm)?;
        #[cfg(windows)]
        {
            if !host_signal::is_valid_signal(signalnum) {
                return Err(vm.new_value_error(format!("signal number {} out of range", signalnum)));
            }
        }
        if !vm.is_main_thread() {
            return Err(vm.new_value_error("signal only works in main thread"));
        }

        let sig_handler =
            match usize::try_from_borrowed_object(vm, &handler).ok() {
                Some(SIG_DFL) => SIG_DFL,
                Some(SIG_IGN) => SIG_IGN,
                None if handler.is_callable() => run_signal as *const () as sighandler_t,
                _ => return Err(vm.new_type_error(
                    "signal handler must be signal.SIG_IGN, signal.SIG_DFL, or a callable object",
                )),
            };
        signal::check_signals(vm)?;

        let old = unsafe { host_signal::install_handler(signalnum, sig_handler) };
        let _old = match old {
            Ok(old) => old,
            Err(_) => {
                return Err(vm.new_os_error("Failed to set signal".to_owned()));
            }
        };

        let signal_handlers = vm.signal_handlers.get_or_init(signal::new_signal_handlers);
        let old_handler = signal_handlers.borrow_mut()[signalnum as usize].replace(handler);
        Ok(old_handler)
    }

    #[pyfunction]
    fn getsignal(signalnum: i32, vm: &VirtualMachine) -> PyResult {
        signal::assert_in_range(signalnum, vm)?;
        let signal_handlers = vm.signal_handlers.get_or_init(signal::new_signal_handlers);
        let handler = signal_handlers.borrow()[signalnum as usize]
            .clone()
            .unwrap_or_else(|| vm.ctx.none());
        Ok(handler)
    }

    #[cfg(unix)]
    #[pyfunction]
    fn alarm(time: u32) -> u32 {
        let prev_time = if time == 0 {
            sig_alarm::cancel()
        } else {
            sig_alarm::set(time)
        };
        prev_time.unwrap_or(0)
    }

    #[cfg(unix)]
    #[pyfunction]
    fn pause(vm: &VirtualMachine) -> PyResult<()> {
        unsafe { libc::pause() };
        signal::check_signals(vm)?;
        Ok(())
    }

    #[cfg(unix)]
    #[pyfunction]
    fn setitimer(
        which: i32,
        seconds: ArgIntoFloat,
        interval: OptionalArg<ArgIntoFloat>,
        vm: &VirtualMachine,
    ) -> PyResult<(f64, f64)> {
        let seconds: f64 = seconds.into();
        let interval: f64 = interval.map(|v| v.into()).unwrap_or(0.0);
        let new = libc::itimerval {
            it_value: double_to_timeval(seconds),
            it_interval: double_to_timeval(interval),
        };
        match host_signal::setitimer(which, &new) {
            Ok(old) => Ok(itimerval_to_tuple(&old)),
            Err(err) => {
                let itimer_error = itimer_error(vm);
                Err(vm.new_exception_msg(itimer_error, err.to_string().into()))
            }
        }
    }

    #[cfg(unix)]
    #[pyfunction]
    fn getitimer(which: i32, vm: &VirtualMachine) -> PyResult<(f64, f64)> {
        match host_signal::getitimer(which) {
            Ok(old) => Ok(itimerval_to_tuple(&old)),
            Err(err) => {
                let itimer_error = itimer_error(vm);
                Err(vm.new_exception_msg(itimer_error, err.to_string().into()))
            }
        }
    }

    #[pyfunction]
    fn default_int_handler(
        _signum: PyObjectRef,
        _arg: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()))
    }

    #[derive(FromArgs)]
    struct SetWakeupFdArgs {
        fd: WakeupFd,
        #[pyarg(named, default = true)]
        warn_on_full_buffer: bool,
    }

    #[pyfunction]
    fn set_wakeup_fd(args: SetWakeupFdArgs, vm: &VirtualMachine) -> PyResult<i64> {
        // TODO: implement warn_on_full_buffer
        let _ = args.warn_on_full_buffer;
        #[cfg(windows)]
        let fd = args.fd.0;
        #[cfg(not(windows))]
        let fd = args.fd;

        if !vm.is_main_thread() {
            return Err(vm.new_value_error("set_wakeup_fd only works in main thread"));
        }

        #[cfg(windows)]
        let is_socket = if fd != INVALID_WAKEUP {
            host_signal::wakeup_fd_is_socket(fd).map_err(|err| {
                if err.kind() == std::io::ErrorKind::InvalidInput {
                    vm.new_value_error("invalid fd")
                } else {
                    err.into_pyexception(vm)
                }
            })?
        } else {
            false
        };
        #[cfg(unix)]
        if let Ok(fd) = unsafe { rustpython_host_env::crt_fd::Borrowed::try_borrow_raw(fd) } {
            use nix::fcntl;
            let oflags = fcntl::fcntl(fd, fcntl::F_GETFL).map_err(|e| e.into_pyexception(vm))?;
            let nonblock =
                fcntl::OFlag::from_bits_truncate(oflags).contains(fcntl::OFlag::O_NONBLOCK);
            if !nonblock {
                return Err(vm.new_value_error(format!(
                    "the fd {} must be in non-blocking mode",
                    fd.as_raw()
                )));
            }
        }

        let old_fd = WAKEUP.swap(fd, Ordering::Relaxed);
        #[cfg(windows)]
        WAKEUP_IS_SOCKET.store(is_socket, Ordering::Relaxed);

        #[cfg(windows)]
        {
            if old_fd == INVALID_WAKEUP {
                Ok(-1)
            } else {
                Ok(old_fd as i64)
            }
        }
        #[cfg(not(windows))]
        {
            Ok(old_fd as i64)
        }
    }

    #[cfg(target_os = "linux")]
    #[pyfunction]
    fn pidfd_send_signal(
        pidfd: i32,
        sig: i32,
        siginfo: OptionalArg<PyObjectRef>,
        flags: OptionalArg<u32>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        signal::assert_in_range(sig, vm)?;
        if let OptionalArg::Present(obj) = siginfo
            && !vm.is_none(&obj)
        {
            return Err(vm.new_type_error("siginfo must be None"));
        }

        let flags = flags.unwrap_or(0);
        host_signal::pidfd_send_signal(pidfd, sig, flags).map_err(|_| vm.new_last_errno_error())
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyfunction(name = "siginterrupt")]
    fn py_siginterrupt(signum: i32, flag: i32, vm: &VirtualMachine) -> PyResult<()> {
        signal::assert_in_range(signum, vm)?;
        host_signal::siginterrupt(signum, flag).map_err(|_| vm.new_last_errno_error())
    }

    /// CPython: signal_raise_signal (signalmodule.c)
    #[cfg(any(unix, windows))]
    #[pyfunction]
    fn raise_signal(signalnum: i32, vm: &VirtualMachine) -> PyResult<()> {
        signal::assert_in_range(signalnum, vm)?;

        // On Windows, only certain signals are supported
        #[cfg(windows)]
        {
            if !host_signal::is_valid_signal(signalnum) {
                return Err(vm
                    .new_errno_error(libc::EINVAL, "Invalid argument")
                    .upcast());
            }
        }

        if host_signal::raise_signal(signalnum).is_err() {
            return Err(vm.new_os_error(format!("raise_signal failed for signal {}", signalnum)));
        }

        // Check if a signal was triggered and handle it
        signal::check_signals(vm)?;

        Ok(())
    }

    /// CPython: signal_strsignal (signalmodule.c)
    #[cfg(unix)]
    #[pyfunction]
    fn strsignal(signalnum: i32, vm: &VirtualMachine) -> PyResult<Option<String>> {
        if signalnum < 1 || signalnum >= signal::NSIG as i32 {
            return Err(vm.new_value_error(format!("signal number {} out of range", signalnum)));
        }
        Ok(host_signal::strsignal(signalnum))
    }

    #[cfg(windows)]
    #[pyfunction]
    fn strsignal(signalnum: i32, vm: &VirtualMachine) -> PyResult<Option<String>> {
        if signalnum < 1 || signalnum >= signal::NSIG as i32 {
            return Err(vm.new_value_error(format!("signal number {} out of range", signalnum)));
        }
        Ok(host_signal::strsignal(signalnum))
    }

    /// CPython: signal_valid_signals (signalmodule.c)
    #[pyfunction]
    fn valid_signals(vm: &VirtualMachine) -> PyResult {
        use crate::PyPayload;
        use crate::builtins::PySet;
        let set = PySet::default().into_ref(&vm.ctx);
        #[cfg(any(unix, windows))]
        for signum in host_signal::valid_signals(signal::NSIG)
            .map_err(|_| vm.new_os_error("sigfillset failed".to_owned()))?
        {
            set.add(vm.ctx.new_int(signum).into(), vm)?;
        }
        #[cfg(not(any(unix, windows)))]
        {
            // Empty set for platforms without signal support (e.g., WASM)
            let _ = &set;
        }
        Ok(set.into())
    }

    #[cfg(unix)]
    fn sigset_to_pyset(mask: &libc::sigset_t, vm: &VirtualMachine) -> PyResult {
        use crate::PyPayload;
        use crate::builtins::PySet;
        let set = PySet::default().into_ref(&vm.ctx);
        for signum in 1..signal::NSIG {
            // SAFETY: mask is a valid sigset_t
            if unsafe { libc::sigismember(mask, signum as i32) } == 1 {
                set.add(vm.ctx.new_int(signum as i32).into(), vm)?;
            }
        }
        Ok(set.into())
    }

    #[cfg(unix)]
    #[pyfunction]
    fn pthread_sigmask(
        how: i32,
        mask: crate::function::ArgIterable,
        vm: &VirtualMachine,
    ) -> PyResult {
        use crate::convert::IntoPyException;

        // Initialize sigset
        let mut sigset = host_signal::sigemptyset().map_err(|e| e.into_pyexception(vm))?;

        // Add signals to the set
        for sig in mask.iter(vm)? {
            let sig = sig?;
            // Convert to i32, handling overflow by returning ValueError
            let signum: i32 = sig.try_to_value(vm).map_err(|_| {
                vm.new_value_error(format!(
                    "signal number out of range [1, {}]",
                    signal::NSIG - 1
                ))
            })?;
            // Validate signal number is in range [1, NSIG)
            if signum < 1 || signum >= signal::NSIG as i32 {
                return Err(vm.new_value_error(format!(
                    "signal number {} out of range [1, {}]",
                    signum,
                    signal::NSIG - 1
                )));
            }
            host_signal::sigaddset(&mut sigset, signum).map_err(|e| e.into_pyexception(vm))?;
        }

        let old_mask =
            host_signal::pthread_sigmask(how, &sigset).map_err(|e| e.into_pyexception(vm))?;

        // Check for pending signals
        signal::check_signals(vm)?;

        // Convert old mask to Python set
        sigset_to_pyset(&old_mask, vm)
    }

    #[cfg(any(unix, windows))]
    pub extern "C" fn run_signal(signum: i32) {
        signal::TRIGGERS[signum as usize].store(true, Ordering::Relaxed);
        signal::set_triggered();
        #[cfg(windows)]
        host_signal::notify_signal(
            signum,
            WAKEUP.load(Ordering::Relaxed),
            WAKEUP_IS_SOCKET.load(Ordering::Relaxed),
            signal::get_sigint_event(),
        );
        #[cfg(unix)]
        host_signal::notify_signal(signum, WAKEUP.load(Ordering::Relaxed));
    }

    /// Reset wakeup fd after fork in child process.
    /// The child must not write to the parent's wakeup fd.
    #[cfg(unix)]
    pub(crate) fn clear_wakeup_fd_after_fork() {
        WAKEUP.store(INVALID_WAKEUP, Ordering::Relaxed);
    }

    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::builtins::PyModule>,
    ) -> PyResult<()> {
        __module_exec(vm, module);

        #[cfg(any(unix, windows))]
        init_signal_handlers(module, vm);

        Ok(())
    }
}
