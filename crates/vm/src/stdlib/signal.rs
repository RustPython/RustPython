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
    use libc::sighandler_t;
    #[allow(non_camel_case_types)]
    #[cfg(not(any(unix, windows)))]
    type sighandler_t = usize;

    cfg_if::cfg_if! {
        if #[cfg(windows)] {
            type WakeupFdRaw = libc::SOCKET;
            struct WakeupFd(WakeupFdRaw);
            const INVALID_WAKEUP: libc::SOCKET = windows_sys::Win32::Networking::WinSock::INVALID_SOCKET;
            static WAKEUP: atomic::AtomicUsize = atomic::AtomicUsize::new(INVALID_WAKEUP);
            // windows doesn't use the same fds for files and sockets like windows does, so we need
            // this to know whether to send() or write()
            static WAKEUP_IS_SOCKET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
        } else {
            type WakeupFdRaw = i32;
            type WakeupFd = WakeupFdRaw;
            const INVALID_WAKEUP: WakeupFd = -1;
            static WAKEUP: atomic::AtomicI32 = atomic::AtomicI32::new(INVALID_WAKEUP);
        }
    }

    #[cfg(unix)]
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

    #[cfg(all(unix, not(target_os = "redox")))]
    unsafe extern "C" {
        fn siginterrupt(sig: i32, flag: i32) -> i32;
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    mod ffi {
        unsafe extern "C" {
            pub fn getitimer(which: libc::c_int, curr_value: *mut libc::itimerval) -> libc::c_int;
            pub fn setitimer(
                which: libc::c_int,
                new_value: *const libc::itimerval,
                old_value: *mut libc::itimerval,
            ) -> libc::c_int;
        }
    }

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
                let handler = unsafe { libc::signal(signum as i32, SIG_IGN) };
                if handler != SIG_ERR {
                    unsafe { libc::signal(signum as i32, handler) };
                }
                let py_handler = if handler == SIG_DFL {
                    Some(sig_dfl.clone())
                } else if handler == SIG_IGN {
                    Some(sig_ign.clone())
                } else {
                    None
                };
                vm.signal_handlers.as_deref().unwrap().borrow_mut()[signum] = py_handler;
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
            const VALID_SIGNALS: &[i32] = &[
                libc::SIGINT,
                libc::SIGILL,
                libc::SIGFPE,
                libc::SIGSEGV,
                libc::SIGTERM,
                SIGBREAK,
                libc::SIGABRT,
            ];
            if !VALID_SIGNALS.contains(&signalnum) {
                return Err(vm.new_value_error(format!("signal number {} out of range", signalnum)));
            }
        }
        let signal_handlers = vm
            .signal_handlers
            .as_deref()
            .ok_or_else(|| vm.new_value_error("signal only works in main thread"))?;

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

        let old = unsafe { libc::signal(signalnum, sig_handler) };
        if old == SIG_ERR {
            return Err(vm.new_os_error("Failed to set signal".to_owned()));
        }
        #[cfg(all(unix, not(target_os = "redox")))]
        unsafe {
            siginterrupt(signalnum, 1);
        }

        let old_handler = signal_handlers.borrow_mut()[signalnum as usize].replace(handler);
        Ok(old_handler)
    }

    #[pyfunction]
    fn getsignal(signalnum: i32, vm: &VirtualMachine) -> PyResult {
        signal::assert_in_range(signalnum, vm)?;
        let signal_handlers = vm
            .signal_handlers
            .as_deref()
            .ok_or_else(|| vm.new_value_error("getsignal only works in main thread"))?;
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
    fn timeval_to_double(tv: &libc::timeval) -> f64 {
        tv.tv_sec as f64 + (tv.tv_usec as f64 / 1_000_000.0)
    }

    #[cfg(unix)]
    fn double_to_timeval(val: f64) -> libc::timeval {
        libc::timeval {
            tv_sec: val.trunc() as _,
            tv_usec: ((val.fract()) * 1_000_000.0) as _,
        }
    }

    #[cfg(unix)]
    fn itimerval_to_tuple(it: &libc::itimerval) -> (f64, f64) {
        (
            timeval_to_double(&it.it_value),
            timeval_to_double(&it.it_interval),
        )
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
        let mut old = core::mem::MaybeUninit::<libc::itimerval>::uninit();
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let ret = unsafe { ffi::setitimer(which, &new, old.as_mut_ptr()) };
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let ret = unsafe { libc::setitimer(which, &new, old.as_mut_ptr()) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            let itimer_error = itimer_error(vm);
            return Err(vm.new_exception_msg(itimer_error, err.to_string()));
        }
        let old = unsafe { old.assume_init() };
        Ok(itimerval_to_tuple(&old))
    }

    #[cfg(unix)]
    #[pyfunction]
    fn getitimer(which: i32, vm: &VirtualMachine) -> PyResult<(f64, f64)> {
        let mut old = core::mem::MaybeUninit::<libc::itimerval>::uninit();
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let ret = unsafe { ffi::getitimer(which, old.as_mut_ptr()) };
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let ret = unsafe { libc::getitimer(which, old.as_mut_ptr()) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            let itimer_error = itimer_error(vm);
            return Err(vm.new_exception_msg(itimer_error, err.to_string()));
        }
        let old = unsafe { old.assume_init() };
        Ok(itimerval_to_tuple(&old))
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

        if vm.signal_handlers.is_none() {
            return Err(vm.new_value_error("signal only works in main thread"));
        }

        #[cfg(windows)]
        let is_socket = if fd != INVALID_WAKEUP {
            use windows_sys::Win32::Networking::WinSock;

            crate::windows::init_winsock();
            let mut res = 0i32;
            let mut res_size = std::mem::size_of::<i32>() as i32;
            let res = unsafe {
                WinSock::getsockopt(
                    fd,
                    WinSock::SOL_SOCKET,
                    WinSock::SO_ERROR,
                    &mut res as *mut i32 as *mut _,
                    &mut res_size,
                )
            };
            // if getsockopt succeeded, fd is for sure a socket
            let is_socket = res == 0;
            if !is_socket {
                let err = std::io::Error::last_os_error();
                // if getsockopt failed for some other reason, throw
                if err.raw_os_error() != Some(WinSock::WSAENOTSOCK) {
                    return Err(err.into_pyexception(vm));
                }
                // Validate that fd is a valid file descriptor using fstat
                // First check if SOCKET can be safely cast to i32 (file descriptor)
                let fd_i32 =
                    i32::try_from(fd).map_err(|_| vm.new_value_error("invalid fd".to_owned()))?;
                // Verify the fd is valid by trying to fstat it
                let borrowed_fd =
                    unsafe { crate::common::crt_fd::Borrowed::try_borrow_raw(fd_i32) }
                        .map_err(|e| e.into_pyexception(vm))?;
                crate::common::fileutils::fstat(borrowed_fd).map_err(|e| e.into_pyexception(vm))?;
            }
            is_socket
        } else {
            false
        };
        #[cfg(unix)]
        if let Ok(fd) = unsafe { crate::common::crt_fd::Borrowed::try_borrow_raw(fd) } {
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
            return Err(vm.new_type_error("siginfo must be None".to_owned()));
        }

        let flags = flags.unwrap_or(0);
        let ret = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd,
                sig,
                std::ptr::null::<libc::siginfo_t>(),
                flags,
            ) as libc::c_long
        };

        if ret == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(())
        }
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyfunction(name = "siginterrupt")]
    fn py_siginterrupt(signum: i32, flag: i32, vm: &VirtualMachine) -> PyResult<()> {
        signal::assert_in_range(signum, vm)?;
        let res = unsafe { siginterrupt(signum, flag) };
        if res < 0 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(())
        }
    }

    /// CPython: signal_raise_signal (signalmodule.c)
    #[cfg(any(unix, windows))]
    #[pyfunction]
    fn raise_signal(signalnum: i32, vm: &VirtualMachine) -> PyResult<()> {
        signal::assert_in_range(signalnum, vm)?;

        // On Windows, only certain signals are supported
        #[cfg(windows)]
        {
            // Windows supports: SIGINT(2), SIGILL(4), SIGFPE(8), SIGSEGV(11), SIGTERM(15), SIGBREAK(21), SIGABRT(22)
            const VALID_SIGNALS: &[i32] = &[
                libc::SIGINT,
                libc::SIGILL,
                libc::SIGFPE,
                libc::SIGSEGV,
                libc::SIGTERM,
                SIGBREAK,
                libc::SIGABRT,
            ];
            if !VALID_SIGNALS.contains(&signalnum) {
                return Err(vm
                    .new_errno_error(libc::EINVAL, "Invalid argument")
                    .upcast());
            }
        }

        let res = unsafe { libc::raise(signalnum) };
        if res != 0 {
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
        let s = unsafe { libc::strsignal(signalnum) };
        if s.is_null() {
            Ok(None)
        } else {
            let cstr = unsafe { core::ffi::CStr::from_ptr(s) };
            Ok(Some(cstr.to_string_lossy().into_owned()))
        }
    }

    #[cfg(windows)]
    #[pyfunction]
    fn strsignal(signalnum: i32, vm: &VirtualMachine) -> PyResult<Option<String>> {
        if signalnum < 1 || signalnum >= signal::NSIG as i32 {
            return Err(vm.new_value_error(format!("signal number {} out of range", signalnum)));
        }
        // Windows doesn't have strsignal(), provide our own mapping
        let name = match signalnum {
            libc::SIGINT => "Interrupt",
            libc::SIGILL => "Illegal instruction",
            libc::SIGFPE => "Floating-point exception",
            libc::SIGSEGV => "Segmentation fault",
            libc::SIGTERM => "Terminated",
            SIGBREAK => "Break",
            libc::SIGABRT => "Aborted",
            _ => return Ok(None),
        };
        Ok(Some(name.to_owned()))
    }

    /// CPython: signal_valid_signals (signalmodule.c)
    #[pyfunction]
    fn valid_signals(vm: &VirtualMachine) -> PyResult {
        use crate::PyPayload;
        use crate::builtins::PySet;
        let set = PySet::default().into_ref(&vm.ctx);
        #[cfg(unix)]
        {
            // Use sigfillset to get all valid signals
            let mut mask: libc::sigset_t = unsafe { core::mem::zeroed() };
            // SAFETY: mask is a valid pointer
            if unsafe { libc::sigfillset(&mut mask) } != 0 {
                return Err(vm.new_os_error("sigfillset failed".to_owned()));
            }
            // Convert the filled mask to a Python set
            for signum in 1..signal::NSIG {
                if unsafe { libc::sigismember(&mask, signum as i32) } == 1 {
                    set.add(vm.ctx.new_int(signum as i32).into(), vm)?;
                }
            }
        }
        #[cfg(windows)]
        {
            // Windows only supports a limited set of signals
            for &signum in &[
                libc::SIGINT,
                libc::SIGILL,
                libc::SIGFPE,
                libc::SIGSEGV,
                libc::SIGTERM,
                SIGBREAK,
                libc::SIGABRT,
            ] {
                set.add(vm.ctx.new_int(signum).into(), vm)?;
            }
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
        let mut sigset: libc::sigset_t = unsafe { core::mem::zeroed() };
        // SAFETY: sigset is a valid pointer
        if unsafe { libc::sigemptyset(&mut sigset) } != 0 {
            return Err(std::io::Error::last_os_error().into_pyexception(vm));
        }

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
            // SAFETY: sigset is a valid pointer and signum is validated
            if unsafe { libc::sigaddset(&mut sigset, signum) } != 0 {
                return Err(std::io::Error::last_os_error().into_pyexception(vm));
            }
        }

        // Call pthread_sigmask
        let mut old_mask: libc::sigset_t = unsafe { core::mem::zeroed() };
        // SAFETY: all pointers are valid
        let err = unsafe { libc::pthread_sigmask(how, &sigset, &mut old_mask) };
        if err != 0 {
            return Err(std::io::Error::from_raw_os_error(err).into_pyexception(vm));
        }

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
        if signum == libc::SIGINT
            && let Some(handle) = signal::get_sigint_event()
        {
            unsafe {
                windows_sys::Win32::System::Threading::SetEvent(handle as _);
            }
        }
        let wakeup_fd = WAKEUP.load(Ordering::Relaxed);
        if wakeup_fd != INVALID_WAKEUP {
            let sigbyte = signum as u8;
            #[cfg(windows)]
            if WAKEUP_IS_SOCKET.load(Ordering::Relaxed) {
                let _res = unsafe {
                    windows_sys::Win32::Networking::WinSock::send(
                        wakeup_fd,
                        &sigbyte as *const u8 as *const _,
                        1,
                        0,
                    )
                };
                return;
            }
            let _res = unsafe { libc::write(wakeup_fd as _, &sigbyte as *const u8 as *const _, 1) };
            // TODO: handle _res < 1, support warn_on_full_buffer
        }
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
