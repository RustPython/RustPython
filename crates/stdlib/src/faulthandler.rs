pub(crate) use decl::make_module;

#[pymodule(name = "faulthandler")]
mod decl {
    use crate::vm::{
        PyObjectRef, PyResult, VirtualMachine, frame::Frame, function::OptionalArg, py_io::Write,
    };
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

    static ENABLED: AtomicBool = AtomicBool::new(false);
    #[allow(dead_code)]
    static FATAL_ERROR_FD: AtomicI32 = AtomicI32::new(2); // stderr by default

    // Fatal signal numbers that should use enable() instead
    #[cfg(unix)]
    const FATAL_SIGNALS: &[i32] = &[
        libc::SIGBUS,
        libc::SIGILL,
        libc::SIGFPE,
        libc::SIGABRT,
        libc::SIGSEGV,
    ];

    const MAX_FUNCTION_NAME_LEN: usize = 500;

    fn truncate_name(name: &str) -> String {
        if name.len() > MAX_FUNCTION_NAME_LEN {
            format!("{}...", &name[..MAX_FUNCTION_NAME_LEN])
        } else {
            name.to_string()
        }
    }

    fn get_file_for_output(
        file: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        match file {
            OptionalArg::Present(f) => {
                // If it's an integer, we can't use it directly as a file object
                // For now, just return it and let the caller handle it
                Ok(f)
            }
            OptionalArg::Missing => {
                // Get sys.stderr
                let stderr = vm.sys_module.get_attr("stderr", vm)?;
                if vm.is_none(&stderr) {
                    return Err(vm.new_runtime_error("sys.stderr is None".to_owned()));
                }
                Ok(stderr)
            }
        }
    }

    fn collect_frame_info(frame: &crate::vm::PyRef<Frame>) -> String {
        let func_name = truncate_name(frame.code.obj_name.as_str());
        format!(
            "  File \"{}\", line {} in {}",
            frame.code.source_path,
            frame.current_location().line,
            func_name
        )
    }

    #[derive(FromArgs)]
    struct DumpTracebackArgs {
        #[pyarg(any, default)]
        file: OptionalArg<PyObjectRef>,
        #[pyarg(any, default = true)]
        all_threads: bool,
    }

    #[pyfunction]
    fn dump_traceback(args: DumpTracebackArgs, vm: &VirtualMachine) -> PyResult<()> {
        let _ = args.all_threads; // TODO: implement all_threads support

        let file = get_file_for_output(args.file, vm)?;

        // Collect frame info first to avoid RefCell borrow conflict
        let frame_lines: Vec<String> = vm.frames.borrow().iter().map(collect_frame_info).collect();

        // Now write to file (in reverse order - most recent call first)
        let mut writer = crate::vm::py_io::PyWriter(file, vm);
        writeln!(writer, "Stack (most recent call first):")?;
        for line in frame_lines.iter().rev() {
            writeln!(writer, "{}", line)?;
        }
        Ok(())
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct EnableArgs {
        #[pyarg(any, default)]
        file: OptionalArg<PyObjectRef>,
        #[pyarg(any, default = true)]
        all_threads: bool,
    }

    #[pyfunction]
    fn enable(args: EnableArgs, vm: &VirtualMachine) -> PyResult<()> {
        // Check that file is valid (if provided) or sys.stderr is not None
        let _file = get_file_for_output(args.file, vm)?;

        ENABLED.store(true, Ordering::Relaxed);

        // Install signal handlers for fatal errors
        #[cfg(not(target_arch = "wasm32"))]
        {
            install_fatal_handlers(vm);
        }

        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn install_fatal_handlers(_vm: &VirtualMachine) {
        // TODO: Install actual signal handlers for SIGSEGV, SIGFPE, etc.
        // This requires careful handling because signal handlers have limited capabilities.
        // For now, this is a placeholder that marks the module as enabled.
    }

    #[pyfunction]
    fn disable() -> bool {
        let was_enabled = ENABLED.swap(false, Ordering::Relaxed);

        // Restore default signal handlers
        #[cfg(not(target_arch = "wasm32"))]
        {
            uninstall_fatal_handlers();
        }

        was_enabled
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn uninstall_fatal_handlers() {
        // TODO: Restore original signal handlers
    }

    #[pyfunction]
    fn is_enabled() -> bool {
        ENABLED.load(Ordering::Relaxed)
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct DumpTracebackLaterArgs {
        #[pyarg(positional)]
        timeout: f64,
        #[pyarg(any, default = false)]
        repeat: bool,
        #[pyarg(any, default)]
        file: OptionalArg<PyObjectRef>,
        #[pyarg(any, default = false)]
        exit: bool,
    }

    #[pyfunction]
    fn dump_traceback_later(args: DumpTracebackLaterArgs, vm: &VirtualMachine) -> PyResult<()> {
        // Check that file is valid
        let _file = get_file_for_output(args.file, vm)?;

        // TODO: Implement watchdog thread that dumps traceback after timeout
        // For now, this is a stub
        Err(vm.new_not_implemented_error("dump_traceback_later is not yet implemented"))
    }

    #[pyfunction]
    fn cancel_dump_traceback_later() {
        // TODO: Cancel the watchdog thread
        // For now, this is a no-op since dump_traceback_later is not implemented
    }

    #[cfg(unix)]
    mod user_signals {
        use std::sync::Mutex;

        const NSIG: usize = 64;

        #[derive(Clone)]
        pub struct UserSignal {
            pub enabled: bool,
            pub fd: i32,
            #[allow(dead_code)]
            pub all_threads: bool,
            pub chain: bool,
            pub previous: libc::sighandler_t,
        }

        impl Default for UserSignal {
            fn default() -> Self {
                Self {
                    enabled: false,
                    fd: 2, // stderr
                    all_threads: true,
                    chain: false,
                    previous: libc::SIG_DFL,
                }
            }
        }

        static USER_SIGNALS: Mutex<Option<Vec<UserSignal>>> = Mutex::new(None);

        pub fn get_user_signal(signum: usize) -> Option<UserSignal> {
            let guard = USER_SIGNALS.lock().unwrap();
            guard.as_ref().and_then(|v| v.get(signum).cloned())
        }

        pub fn set_user_signal(signum: usize, signal: UserSignal) {
            let mut guard = USER_SIGNALS.lock().unwrap();
            if guard.is_none() {
                *guard = Some(vec![UserSignal::default(); NSIG]);
            }
            if let Some(ref mut v) = *guard
                && signum < v.len()
            {
                v[signum] = signal;
            }
        }

        pub fn clear_user_signal(signum: usize) -> Option<UserSignal> {
            let mut guard = USER_SIGNALS.lock().unwrap();
            if let Some(ref mut v) = *guard
                && signum < v.len()
                && v[signum].enabled
            {
                let old = v[signum].clone();
                v[signum] = UserSignal::default();
                return Some(old);
            }
            None
        }

        pub fn is_enabled(signum: usize) -> bool {
            let guard = USER_SIGNALS.lock().unwrap();
            guard
                .as_ref()
                .and_then(|v| v.get(signum))
                .is_some_and(|s| s.enabled)
        }
    }

    #[cfg(unix)]
    extern "C" fn faulthandler_user_signal(signum: libc::c_int) {
        let user = match user_signals::get_user_signal(signum as usize) {
            Some(u) if u.enabled => u,
            _ => return,
        };

        // Write traceback header
        let header = b"Current thread 0x0000 (most recent call first):\n";
        let _ = unsafe {
            libc::write(
                user.fd,
                header.as_ptr() as *const libc::c_void,
                header.len(),
            )
        };

        // Note: We cannot easily access RustPython's frame stack from a signal handler
        // because signal handlers run asynchronously. We just output a placeholder.
        let msg = b"  <signal handler invoked, traceback unavailable in signal context>\n";
        let _ = unsafe { libc::write(user.fd, msg.as_ptr() as *const libc::c_void, msg.len()) };

        // If chain is enabled, call the previous handler
        if user.chain && user.previous != libc::SIG_DFL && user.previous != libc::SIG_IGN {
            // Re-register the old handler and raise the signal
            unsafe {
                libc::signal(signum, user.previous);
                libc::raise(signum);
                // Re-register our handler
                libc::signal(signum, faulthandler_user_signal as libc::sighandler_t);
            }
        }
    }

    #[cfg(unix)]
    fn check_signum(signum: i32, vm: &VirtualMachine) -> PyResult<()> {
        // Check if it's a fatal signal
        if FATAL_SIGNALS.contains(&signum) {
            return Err(vm.new_runtime_error(format!(
                "signal {} cannot be registered, use enable() instead",
                signum
            )));
        }

        // Check if signal is in valid range
        if !(1..64).contains(&signum) {
            return Err(vm.new_value_error("signal number out of range".to_owned()));
        }

        Ok(())
    }

    #[cfg(unix)]
    fn get_fd_from_file(file: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<i32> {
        match file {
            OptionalArg::Present(f) => {
                // Check if it's an integer (file descriptor)
                if let Ok(fd) = f.try_to_value::<i32>(vm) {
                    if fd < 0 {
                        return Err(
                            vm.new_value_error("file is not a valid file descriptor".to_owned())
                        );
                    }
                    return Ok(fd);
                }
                // Try to get fileno() from file object
                let fileno = vm.call_method(&f, "fileno", ())?;
                let fd: i32 = fileno.try_to_value(vm)?;
                if fd < 0 {
                    return Err(
                        vm.new_value_error("file is not a valid file descriptor".to_owned())
                    );
                }
                // Try to flush the file
                let _ = vm.call_method(&f, "flush", ());
                Ok(fd)
            }
            OptionalArg::Missing => {
                // Get sys.stderr
                let stderr = vm.sys_module.get_attr("stderr", vm)?;
                if vm.is_none(&stderr) {
                    return Err(vm.new_runtime_error("sys.stderr is None".to_owned()));
                }
                let fileno = vm.call_method(&stderr, "fileno", ())?;
                let fd: i32 = fileno.try_to_value(vm)?;
                let _ = vm.call_method(&stderr, "flush", ());
                Ok(fd)
            }
        }
    }

    #[cfg(unix)]
    #[derive(FromArgs)]
    #[allow(unused)]
    struct RegisterArgs {
        #[pyarg(positional)]
        signum: i32,
        #[pyarg(any, default)]
        file: OptionalArg<PyObjectRef>,
        #[pyarg(any, default = true)]
        all_threads: bool,
        #[pyarg(any, default = false)]
        chain: bool,
    }

    #[cfg(unix)]
    #[pyfunction]
    fn register(args: RegisterArgs, vm: &VirtualMachine) -> PyResult<()> {
        check_signum(args.signum, vm)?;

        let fd = get_fd_from_file(args.file, vm)?;

        let signum = args.signum as usize;

        // Get current handler to save as previous
        let previous = if !user_signals::is_enabled(signum) {
            // Install signal handler
            let prev = unsafe {
                libc::signal(args.signum, faulthandler_user_signal as libc::sighandler_t)
            };
            if prev == libc::SIG_ERR {
                return Err(vm.new_os_error(format!(
                    "Failed to register signal handler for signal {}",
                    args.signum
                )));
            }
            prev
        } else {
            // Already registered, keep previous handler
            user_signals::get_user_signal(signum)
                .map(|u| u.previous)
                .unwrap_or(libc::SIG_DFL)
        };

        user_signals::set_user_signal(
            signum,
            user_signals::UserSignal {
                enabled: true,
                fd,
                all_threads: args.all_threads,
                chain: args.chain,
                previous,
            },
        );

        Ok(())
    }

    #[cfg(unix)]
    #[pyfunction]
    fn unregister(signum: i32, vm: &VirtualMachine) -> PyResult<bool> {
        check_signum(signum, vm)?;

        if let Some(old) = user_signals::clear_user_signal(signum as usize) {
            // Restore previous handler
            unsafe {
                libc::signal(signum, old.previous);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // Test functions for faulthandler testing

    #[pyfunction]
    fn _read_null() {
        // This function intentionally causes a segmentation fault by reading from NULL
        // Used for testing faulthandler
        #[cfg(not(target_arch = "wasm32"))]
        unsafe {
            suppress_crash_report();
            let ptr: *const i32 = std::ptr::null();
            std::ptr::read_volatile(ptr);
        }
    }

    #[derive(FromArgs)]
    #[allow(dead_code)]
    struct SigsegvArgs {
        #[pyarg(any, default = false)]
        release_gil: bool,
    }

    #[pyfunction]
    fn _sigsegv(_args: SigsegvArgs) {
        // Raise SIGSEGV signal
        #[cfg(not(target_arch = "wasm32"))]
        {
            suppress_crash_report();

            // Reset SIGSEGV to default behavior before raising
            // This ensures the process will actually crash
            unsafe {
                libc::signal(libc::SIGSEGV, libc::SIG_DFL);
            }

            #[cfg(windows)]
            {
                // On Windows, we need to raise SIGSEGV multiple times
                loop {
                    unsafe {
                        libc::raise(libc::SIGSEGV);
                    }
                }
            }
            #[cfg(not(windows))]
            unsafe {
                libc::raise(libc::SIGSEGV);
            }
        }
    }

    #[pyfunction]
    fn _sigabrt() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            suppress_crash_report();
            unsafe {
                libc::abort();
            }
        }
    }

    #[pyfunction]
    fn _sigfpe() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            suppress_crash_report();

            // Reset SIGFPE to default behavior before raising
            unsafe {
                libc::signal(libc::SIGFPE, libc::SIG_DFL);
            }

            // Raise SIGFPE
            unsafe {
                libc::raise(libc::SIGFPE);
            }
        }
    }

    #[pyfunction]
    fn _fatal_error_c_thread() {
        // This would call Py_FatalError in a new C thread
        // For RustPython, we just panic in a new thread
        #[cfg(not(target_arch = "wasm32"))]
        {
            suppress_crash_report();
            std::thread::spawn(|| {
                panic!("Fatal Python error: in new thread");
            });
            // Wait a bit for the thread to panic
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn suppress_crash_report() {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Diagnostics::Debug::{
                SEM_NOGPFAULTERRORBOX, SetErrorMode,
            };
            unsafe {
                let mode = SetErrorMode(SEM_NOGPFAULTERRORBOX);
                SetErrorMode(mode | SEM_NOGPFAULTERRORBOX);
            }
        }

        #[cfg(unix)]
        {
            // Disable core dumps
            #[cfg(not(any(target_os = "redox", target_os = "wasi")))]
            {
                use libc::{RLIMIT_CORE, rlimit, setrlimit};
                let rl = rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                unsafe {
                    let _ = setrlimit(RLIMIT_CORE, &rl);
                }
            }
        }
    }

    // Windows-specific constants
    #[cfg(windows)]
    #[pyattr]
    const _EXCEPTION_ACCESS_VIOLATION: u32 = 0xC0000005;

    #[cfg(windows)]
    #[pyattr]
    const _EXCEPTION_INT_DIVIDE_BY_ZERO: u32 = 0xC0000094;

    #[cfg(windows)]
    #[pyattr]
    const _EXCEPTION_STACK_OVERFLOW: u32 = 0xC00000FD;

    #[cfg(windows)]
    #[pyattr]
    const _EXCEPTION_NONCONTINUABLE: u32 = 0x00000001;

    #[cfg(windows)]
    #[pyattr]
    const _EXCEPTION_NONCONTINUABLE_EXCEPTION: u32 = 0xC0000025;

    #[cfg(windows)]
    #[derive(FromArgs)]
    struct RaiseExceptionArgs {
        #[pyarg(positional)]
        code: u32,
        #[pyarg(positional, default = 0)]
        flags: u32,
    }

    #[cfg(windows)]
    #[pyfunction]
    fn _raise_exception(args: RaiseExceptionArgs) {
        use windows_sys::Win32::System::Diagnostics::Debug::RaiseException;

        suppress_crash_report();
        unsafe {
            RaiseException(args.code, args.flags, 0, std::ptr::null());
        }
    }
}
