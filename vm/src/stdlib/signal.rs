use crate::{PyRef, VirtualMachine, builtins::PyModule};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = _signal::make_module(vm);

    #[cfg(any(unix, windows))]
    _signal::init_signal_handlers(&module, vm);

    module
}

#[pymodule]
pub(crate) mod _signal {
    #[cfg(any(unix, windows))]
    use crate::{
        Py,
        convert::{IntoPyException, TryFromBorrowedObject},
    };
    use crate::{PyObjectRef, PyResult, VirtualMachine, signal};
    use std::sync::atomic::{self, Ordering};

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

    #[pyattr]
    use crate::signal::NSIG;

    #[cfg(any(unix, windows))]
    #[pyattr]
    pub use libc::{SIGABRT, SIGFPE, SIGILL, SIGINT, SIGSEGV, SIGTERM};

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

    #[cfg(any(unix, windows))]
    pub(super) fn init_signal_handlers(
        module: &Py<crate::builtins::PyModule>,
        vm: &VirtualMachine,
    ) {
        if vm.state.settings.install_signal_handlers {
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
        Err(vm.new_not_implemented_error("signal is not implemented on this platform".to_owned()))
    }

    #[cfg(any(unix, windows))]
    #[pyfunction]
    pub fn signal(
        signalnum: i32,
        handler: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        signal::assert_in_range(signalnum, vm)?;
        let signal_handlers = vm
            .signal_handlers
            .as_deref()
            .ok_or_else(|| vm.new_value_error("signal only works in main thread".to_owned()))?;

        let sig_handler =
            match usize::try_from_borrowed_object(vm, &handler).ok() {
                Some(SIG_DFL) => SIG_DFL,
                Some(SIG_IGN) => SIG_IGN,
                None if handler.is_callable() => run_signal as sighandler_t,
                _ => return Err(vm.new_type_error(
                    "signal handler must be signal.SIG_IGN, signal.SIG_DFL, or a callable object"
                        .to_owned(),
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

        let old_handler = std::mem::replace(
            &mut signal_handlers.borrow_mut()[signalnum as usize],
            Some(handler),
        );
        Ok(old_handler)
    }

    #[pyfunction]
    fn getsignal(signalnum: i32, vm: &VirtualMachine) -> PyResult {
        signal::assert_in_range(signalnum, vm)?;
        let signal_handlers = vm
            .signal_handlers
            .as_deref()
            .ok_or_else(|| vm.new_value_error("getsignal only works in main thread".to_owned()))?;
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
        #[pyarg(named, default = "true")]
        warn_on_full_buffer: bool,
    }

    #[pyfunction]
    fn set_wakeup_fd(args: SetWakeupFdArgs, vm: &VirtualMachine) -> PyResult<WakeupFdRaw> {
        // TODO: implement warn_on_full_buffer
        let _ = args.warn_on_full_buffer;
        #[cfg(windows)]
        let fd = args.fd.0;
        #[cfg(not(windows))]
        let fd = args.fd;

        if vm.signal_handlers.is_none() {
            return Err(vm.new_value_error("signal only works in main thread".to_owned()));
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
            }
            is_socket
        } else {
            false
        };
        #[cfg(unix)]
        if fd != INVALID_WAKEUP {
            use nix::fcntl;
            let oflags = fcntl::fcntl(fd, fcntl::F_GETFL).map_err(|e| e.into_pyexception(vm))?;
            let nonblock =
                fcntl::OFlag::from_bits_truncate(oflags).contains(fcntl::OFlag::O_NONBLOCK);
            if !nonblock {
                return Err(vm.new_value_error(format!("the fd {fd} must be in non-blocking mode")));
            }
        }

        let old_fd = WAKEUP.swap(fd, Ordering::Relaxed);
        #[cfg(windows)]
        WAKEUP_IS_SOCKET.store(is_socket, Ordering::Relaxed);

        Ok(old_fd)
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyfunction(name = "siginterrupt")]
    fn py_siginterrupt(signum: i32, flag: i32, vm: &VirtualMachine) -> PyResult<()> {
        signal::assert_in_range(signum, vm)?;
        let res = unsafe { siginterrupt(signum, flag) };
        if res < 0 {
            Err(crate::stdlib::os::errno_err(vm))
        } else {
            Ok(())
        }
    }

    #[cfg(any(unix, windows))]
    pub extern "C" fn run_signal(signum: i32) {
        signal::TRIGGERS[signum as usize].store(true, Ordering::Relaxed);
        signal::set_triggered();
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
}
