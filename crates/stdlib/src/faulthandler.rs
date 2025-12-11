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
        // Check that file is valid
        let _file = get_file_for_output(args.file, vm)?;

        // TODO: Register a handler for the given signal
        Err(vm.new_not_implemented_error("register is not yet fully implemented"))
    }

    #[cfg(unix)]
    #[pyfunction]
    fn unregister(signum: i32, vm: &VirtualMachine) -> PyResult<bool> {
        let _ = signum;
        // TODO: Unregister the handler for the given signal
        Err(vm.new_not_implemented_error("unregister is not yet fully implemented"))
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
