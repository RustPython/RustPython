pub(crate) use decl::make_module;

#[pymodule(name = "faulthandler")]
mod decl {
    use std::sync::OnceLock;
    use std::sync::{Arc, atomic::AtomicBool};
    use std::thread;

    use crate::vm::{PyObjectRef, PyResult};
    use num_traits::ToPrimitive;
    use rustpython_common::lock::PyMutex;
    use rustpython_vm::builtins::{PyFloat, PyInt};
    #[cfg(target_os = "windows")]
    use windows_sys::Win32::System::Diagnostics::Debug::{SEM_NOGPFAULTERRORBOX, SetErrorMode};

    use crate::vm::{VirtualMachine, frame::Frame, function::OptionalArg, stdlib::sys::PyStderr};

    fn suppress_crash_report() {
        // On Windows desktops, suppress Windows Error Reporting dialogs.
        #[cfg(target_os = "windows")]
        unsafe {
            let mode = SetErrorMode(SEM_NOGPFAULTERRORBOX);
            SetErrorMode(mode | SEM_NOGPFAULTERRORBOX);
        }

        // On Unix-like systems, disable creation of core dumps.
        #[cfg(unix)]
        unsafe {
            // Requires libc crate
            let mut rl: libc::rlimit = std::mem::zeroed();
            if libc::getrlimit(libc::RLIMIT_CORE, &mut rl) == 0 {
                rl.rlim_cur = 0;
                libc::setrlimit(libc::RLIMIT_CORE, &rl);
            }
        }

        // For MSVC builds on Windows, configure abort behavior so that
        // no error message or fault reporting popup is shown.
        #[cfg(all(target_os = "windows", target_env = "msvc"))]
        unsafe {
            unsafe extern "C" {
                fn _set_abort_behavior(flags: u32, mask: u32) -> u32;
            }
            const _WRITE_ABORT_MSG: u32 = 0x1;
            const _CALL_REPORTFAULT: u32 = 0x2;
            _set_abort_behavior(0, _WRITE_ABORT_MSG | _CALL_REPORTFAULT);
        }
    }

    #[pyfunction]
    fn _read_null() -> i64 {
        suppress_crash_report();
        let x: *const i64 = std::ptr::null();
        // Crash time
        // ensure the compiler does not optimize out the null dereference by returning
        unsafe { *x }
    }

    fn dump_frame(frame: &Frame, vm: &VirtualMachine) {
        let stderr = PyStderr(vm);
        writeln!(
            stderr,
            "  File \"{}\", line {} in {}",
            frame.code.source_path,
            frame.current_location().row,
            frame.code.obj_name
        )
    }

    static LOCK: AtomicBool = AtomicBool::new(false);

    #[pyfunction]
    fn dump_traceback(
        _file: OptionalArg<i64>,
        _all_threads: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) {
        if LOCK.swap(true, std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let stderr = PyStderr(vm);
        writeln!(stderr, "Stack (most recent call first):");

        for frame in vm.frames.borrow().iter() {
            dump_frame(frame, vm);
        }
        LOCK.store(false, std::sync::atomic::Ordering::Release);
    }

    const PY_TIMEOUT_MAX: u64 = 1_000_000_000; // e.g. maximum microseconds timeout
    const SEC_TO_US: u64 = 1_000_000;
    const LONG_MAX: u64 = i64::MAX as u64;

    fn time_from_seconds_object(timeout_obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<u64> {
        // try float
        if let Some(timeout) = timeout_obj.downcast_ref::<PyFloat>() {
            let timeout = timeout.to_f64();
            if timeout < 0.0 {
                return Err(vm.new_value_error("timeout must be greater than 0".to_owned()));
            }
            let timeout_us = (timeout * SEC_TO_US as f64) as u64;
            if timeout_us == 0 {
                return Err(vm.new_value_error("timeout must be greater than 0".to_owned()));
            }
            return Ok(timeout_us);
        }
        // try int
        if let Some(timeout) = timeout_obj.downcast_ref::<PyInt>() {
            let timeout = timeout
                .as_bigint()
                .to_u64()
                .ok_or_else(|| vm.new_overflow_error("timeout value is too large".to_owned()))?;
            return Ok(timeout);
        }
        Err(vm.new_type_error("timeout must be a float or int".to_owned()))
    }

    fn format_timeout(timeout_us: u64) -> Option<String> {
        // Return a header string representing the timeout.
        Some(format!("Timeout in {} us", timeout_us))
    }

    #[pyfunction]
    fn cancel_dump_traceback_later() {
        // Acquire the global watchdog state.
        let watchdog = get_watchdog();
        let mut wd = watchdog.lock();

        // If not scheduled, nothing to cancel.
        if wd.cancel_event.is_none() {
            return;
        }

        // Notify cancellation:
        wd.cancel_event = None;
        drop(wd);

        // Wait for the watchdog thread to finish by acquiring the running lock.
        {
            let watchdog = get_watchdog();
            let wd = watchdog.lock();
            if let Some(ref running) = wd.running {
                // This call blocks until the watchdog thread finishes.
                let lock = running.lock();
                // Immediately drop to “release” the running lock.
                drop(lock);
            }
        }

        // Reacquire cancel_event: the main thread holds this lock after cancellation.
        let mut wd = watchdog.lock();
        let cancel_stop;
        wd.cancel_event = Some({
            let lock = Arc::new(PyMutex::new(()));
            let lock_clone = lock.clone();
            cancel_stop = lock;
            lock_clone
        });

        // Lock it immediately.
        let lock = cancel_stop.lock();

        // Clear file and header.
        wd.file = None;
        wd.header = None;
        wd.header_len = 0;
        drop(lock);
    }

    fn faulthandler_thread() {
        // Take a snapshot of the needed watchdog state.
        let (_fd, _timeout_us, exit, repeat, vm, header, cancel_event, running) = {
            let wd = get_watchdog().lock();
            (
                wd.fd,
                wd.timeout_us,
                wd.exit,
                wd.repeat,
                wd.vm,
                wd.header.clone(),
                wd.cancel_event.clone(),
                wd.running.clone(),
            )
        };

        loop {
            // Try to acquire cancel_event with a timeout.
            let cancelled = if let Some(ref cancel) = cancel_event {
                // TODO: use try_lock_for instead
                if let Some(_guard) = cancel.try_lock() {
                    // Cancel event acquired: thread cancellation was signaled.
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if cancelled {
                break;
            }

            // Timeout occurred: dump traceback.
            if let Some(ref hdr) = header {
                eprintln!("{}", hdr);
            }

            // Safety? what safety?
            let vm: &VirtualMachine = unsafe { &*vm.unwrap() };
            for frame in vm.frames.borrow().iter() {
                dump_frame(frame, vm);
            }
            // TODO: Write to file descriptor (fd).
            let ok = true;

            if exit {
                std::process::exit(1);
            }
            if !ok || repeat == 0 {
                break;
            }
        }

        // Signal thread termination by releasing the running lock.
        if let Some(ref running_lock) = running {
            let lock = running_lock.lock();
            drop(lock);
        }
    }

    // Global state for the watchdog thread.
    struct WatchdogThread {
        running: Option<Arc<PyMutex<()>>>,
        cancel_event: Option<Arc<PyMutex<()>>>,
        file: Option<PyObjectRef>,
        fd: i32,
        timeout_us: u64,
        repeat: i32,
        vm: Option<*const VirtualMachine>,
        exit: bool,
        header: Option<String>,
        header_len: usize,
    }

    unsafe impl Send for WatchdogThread {}
    unsafe impl Sync for WatchdogThread {}

    static WATCHDOG: OnceLock<PyMutex<WatchdogThread>> = OnceLock::new();

    fn get_watchdog() -> &'static PyMutex<WatchdogThread> {
        WATCHDOG.get_or_init(|| {
            PyMutex::new(WatchdogThread {
                running: None,
                cancel_event: None,
                file: None,
                fd: -1 as _,
                timeout_us: 0,
                repeat: 0,
                vm: None,
                exit: false,
                header: None,
                header_len: 0,
            })
        });
        WATCHDOG.get().unwrap()
    }

    #[pyfunction]
    fn dump_traceback_later(
        timeout_obj: PyObjectRef,
        repeat: Option<i32>,
        file: Option<PyObjectRef>,
        exit: Option<bool>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Convert timeout_obj to microseconds.
        let timeout_us = time_from_seconds_object(&timeout_obj, vm)?;
        if timeout_us == 0 {
            return Err(vm.new_value_error("timeout must be greater than 0".to_owned()));
        }
        if timeout_us > PY_TIMEOUT_MAX || timeout_us / SEC_TO_US > LONG_MAX {
            return Err(vm.new_overflow_error("timeout value is too large".to_owned()));
        }

        // TODO: Extract file descriptor.
        // let fd = faulthandler_get_fileno(file.clone(), vm)?;
        // if fd < 0 {
        // return Err(vm.new_runtime_error("invalid file descriptor".to_owned()));
        // }

        // Format the timeout header.
        let header = format_timeout(timeout_us)
            .ok_or_else(|| vm.new_memory_error("failed to allocate header".to_owned()))?;
        let header_len = header.len();

        {
            // Get global watchdog state.
            let mut watchdog = get_watchdog().lock();

            // Initialize locks if not already done.
            if watchdog.running.is_none() {
                watchdog.running = Some(Arc::new(PyMutex::new(())));
            }
            if watchdog.cancel_event.is_none() {
                // The cancel_event lock is acquired immediately for later cancellation.
                let lock = Arc::new(PyMutex::new(()));
                let lock_clone = lock.clone();
                // Lock it immediately.
                let _guard = lock.lock();
                watchdog.cancel_event = Some(lock_clone);
            }

            // Cancel previous watchdog thread if running.
            cancel_dump_traceback_later();

            // Set watchdog thread parameters.
            watchdog.file = file;
            watchdog.fd = 0; // Placeholder for file descriptor
            watchdog.timeout_us = timeout_us;
            watchdog.repeat = repeat.unwrap_or(0);
            // This is a placeholder; adjust as needed to extract interpreter data.
            watchdog.vm = Some(vm);
            watchdog.exit = exit.unwrap_or(false);
            watchdog.header = Some(header);
            watchdog.header_len = header_len;

            // "Arm" the running lock.
            if let Some(ref running_lock) = watchdog.running {
                let lock = running_lock.lock();
                drop(lock);
            }
        }

        // Spawn the watchdog thread.
        let thread_handle = thread::Builder::new()
            .name("faulthandler watchdog".to_owned())
            .spawn(|| {
                faulthandler_thread();
            });

        if thread_handle.is_err() {
            // On failure, cleanup and report error.
            {
                let mut watchdog = get_watchdog().lock();
                watchdog.file = None;
                // Ideally, also reset header and other fields.
            }
            return Err(vm.new_runtime_error("unable to start watchdog thread".to_owned()));
        }

        Ok(())
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct EnableArgs {
        #[pyarg(any, default)]
        file: Option<i64>,
        #[pyarg(any, default = true)]
        all_threads: bool,
    }

    #[pyfunction]
    fn enable(_args: EnableArgs) {
        // TODO
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct RegisterArgs {
        #[pyarg(positional)]
        signum: i64,
        #[pyarg(any, default)]
        file: Option<i64>,
        #[pyarg(any, default = true)]
        all_threads: bool,
        #[pyarg(any, default = false)]
        chain: bool,
    }

    #[pyfunction]
    fn register(_args: RegisterArgs) {
        // TODO
    }

    #[pyfunction]
    fn _sigabrt() {
        suppress_crash_report();
    }
}
