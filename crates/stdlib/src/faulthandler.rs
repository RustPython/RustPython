pub(crate) use decl::module_def;

#[allow(static_mut_refs)] // TODO: group code only with static mut refs
#[pymodule(name = "faulthandler")]
mod decl {
    use crate::vm::{
        PyObjectRef, PyResult, VirtualMachine,
        frame::Frame,
        function::{ArgIntoFloat, OptionalArg},
        py_io::Write,
    };
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use core::time::Duration;
    use parking_lot::{Condvar, Mutex};
    #[cfg(any(unix, windows))]
    use rustpython_common::os::{get_errno, set_errno};
    use std::thread;

    /// fault_handler_t
    #[cfg(unix)]
    struct FaultHandler {
        signum: libc::c_int,
        enabled: bool,
        name: &'static str,
        previous: libc::sigaction,
    }

    #[cfg(windows)]
    struct FaultHandler {
        signum: libc::c_int,
        enabled: bool,
        name: &'static str,
        previous: libc::sighandler_t,
    }

    #[cfg(unix)]
    impl FaultHandler {
        const fn new(signum: libc::c_int, name: &'static str) -> Self {
            Self {
                signum,
                enabled: false,
                name,
                // SAFETY: sigaction is a C struct that can be zero-initialized
                previous: unsafe { core::mem::zeroed() },
            }
        }
    }

    #[cfg(windows)]
    impl FaultHandler {
        const fn new(signum: libc::c_int, name: &'static str) -> Self {
            Self {
                signum,
                enabled: false,
                name,
                previous: 0,
            }
        }
    }

    /// faulthandler_handlers[]
    /// Number of fatal signals
    #[cfg(unix)]
    const FAULTHANDLER_NSIGNALS: usize = 5;
    #[cfg(windows)]
    const FAULTHANDLER_NSIGNALS: usize = 4;

    // CPython uses static arrays for signal handlers which requires mutable static access.
    // This is safe because:
    // 1. Signal handlers run in a single-threaded context (from the OS perspective)
    // 2. FAULTHANDLER_HANDLERS is only modified during enable/disable operations
    // 3. This matches CPython's faulthandler.c implementation
    #[cfg(unix)]
    static mut FAULTHANDLER_HANDLERS: [FaultHandler; FAULTHANDLER_NSIGNALS] = [
        FaultHandler::new(libc::SIGBUS, "Bus error"),
        FaultHandler::new(libc::SIGILL, "Illegal instruction"),
        FaultHandler::new(libc::SIGFPE, "Floating-point exception"),
        FaultHandler::new(libc::SIGABRT, "Aborted"),
        FaultHandler::new(libc::SIGSEGV, "Segmentation fault"),
    ];

    #[cfg(windows)]
    static mut FAULTHANDLER_HANDLERS: [FaultHandler; FAULTHANDLER_NSIGNALS] = [
        FaultHandler::new(libc::SIGILL, "Illegal instruction"),
        FaultHandler::new(libc::SIGFPE, "Floating-point exception"),
        FaultHandler::new(libc::SIGABRT, "Aborted"),
        FaultHandler::new(libc::SIGSEGV, "Segmentation fault"),
    ];

    /// fatal_error state
    struct FatalErrorState {
        enabled: AtomicBool,
        fd: AtomicI32,
        all_threads: AtomicBool,
    }

    static FATAL_ERROR: FatalErrorState = FatalErrorState {
        enabled: AtomicBool::new(false),
        fd: AtomicI32::new(2), // stderr by default
        all_threads: AtomicBool::new(true),
    };

    // Watchdog thread state for dump_traceback_later
    struct WatchdogState {
        cancel: bool,
        fd: i32,
        timeout_us: u64,
        repeat: bool,
        exit: bool,
        header: String,
    }

    type WatchdogHandle = Arc<(Mutex<WatchdogState>, Condvar)>;
    static WATCHDOG: Mutex<Option<WatchdogHandle>> = Mutex::new(None);

    // Frame snapshot for signal-safe traceback (RustPython-specific)

    /// Frame information snapshot for signal-safe access
    #[cfg(any(unix, windows))]
    #[derive(Clone, Copy)]
    struct FrameSnapshot {
        filename: [u8; 256],
        filename_len: usize,
        lineno: u32,
        funcname: [u8; 128],
        funcname_len: usize,
    }

    #[cfg(any(unix, windows))]
    impl FrameSnapshot {
        const EMPTY: Self = Self {
            filename: [0; 256],
            filename_len: 0,
            lineno: 0,
            funcname: [0; 128],
            funcname_len: 0,
        };
    }

    #[cfg(any(unix, windows))]
    const MAX_SNAPSHOT_FRAMES: usize = 100;

    /// Signal-safe global storage for frame snapshots
    #[cfg(any(unix, windows))]
    static mut FRAME_SNAPSHOTS: [FrameSnapshot; MAX_SNAPSHOT_FRAMES] =
        [FrameSnapshot::EMPTY; MAX_SNAPSHOT_FRAMES];
    #[cfg(any(unix, windows))]
    static SNAPSHOT_COUNT: core::sync::atomic::AtomicUsize =
        core::sync::atomic::AtomicUsize::new(0);

    // Signal-safe output functions

    // PUTS macro
    #[cfg(any(unix, windows))]
    fn puts(fd: i32, s: &str) {
        let _ = unsafe {
            #[cfg(windows)]
            {
                libc::write(fd, s.as_ptr() as *const libc::c_void, s.len() as u32)
            }
            #[cfg(not(windows))]
            {
                libc::write(fd, s.as_ptr() as *const libc::c_void, s.len())
            }
        };
    }

    // _Py_DumpHexadecimal (traceback.c)
    #[cfg(any(unix, windows))]
    fn dump_hexadecimal(fd: i32, value: u64, width: usize) {
        const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
        let mut buf = [0u8; 18]; // "0x" + 16 hex digits
        buf[0] = b'0';
        buf[1] = b'x';

        for i in 0..width {
            let digit = ((value >> (4 * (width - 1 - i))) & 0xf) as usize;
            buf[2 + i] = HEX_CHARS[digit];
        }

        let _ = unsafe {
            #[cfg(windows)]
            {
                libc::write(fd, buf.as_ptr() as *const libc::c_void, (2 + width) as u32)
            }
            #[cfg(not(windows))]
            {
                libc::write(fd, buf.as_ptr() as *const libc::c_void, 2 + width)
            }
        };
    }

    // _Py_DumpDecimal (traceback.c)
    #[cfg(any(unix, windows))]
    fn dump_decimal(fd: i32, value: usize) {
        let mut buf = [0u8; 20];
        let mut v = value;
        let mut i = buf.len();

        if v == 0 {
            puts(fd, "0");
            return;
        }

        while v > 0 {
            i -= 1;
            buf[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }

        let len = buf.len() - i;
        let _ = unsafe {
            #[cfg(windows)]
            {
                libc::write(fd, buf[i..].as_ptr() as *const libc::c_void, len as u32)
            }
            #[cfg(not(windows))]
            {
                libc::write(fd, buf[i..].as_ptr() as *const libc::c_void, len)
            }
        };
    }

    /// Get current thread ID
    #[cfg(unix)]
    fn current_thread_id() -> u64 {
        unsafe { libc::pthread_self() as u64 }
    }

    #[cfg(windows)]
    fn current_thread_id() -> u64 {
        unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() as u64 }
    }

    // write_thread_id (traceback.c:1240-1256)
    #[cfg(any(unix, windows))]
    fn write_thread_id(fd: i32, is_current: bool) {
        if is_current {
            puts(fd, "Current thread 0x");
        } else {
            puts(fd, "Thread 0x");
        }
        let thread_id = current_thread_id();
        // Use appropriate width based on platform pointer size
        dump_hexadecimal(fd, thread_id, core::mem::size_of::<usize>() * 2);
        puts(fd, " (most recent call first):\n");
    }

    // dump_frame (traceback.c:1037-1087)
    #[cfg(any(unix, windows))]
    fn dump_frame(fd: i32, filename: &[u8], lineno: u32, funcname: &[u8]) {
        puts(fd, "  File \"");
        let _ = unsafe {
            #[cfg(windows)]
            {
                libc::write(
                    fd,
                    filename.as_ptr() as *const libc::c_void,
                    filename.len() as u32,
                )
            }
            #[cfg(not(windows))]
            {
                libc::write(fd, filename.as_ptr() as *const libc::c_void, filename.len())
            }
        };
        puts(fd, "\", line ");
        dump_decimal(fd, lineno as usize);
        puts(fd, " in ");
        let _ = unsafe {
            #[cfg(windows)]
            {
                libc::write(
                    fd,
                    funcname.as_ptr() as *const libc::c_void,
                    funcname.len() as u32,
                )
            }
            #[cfg(not(windows))]
            {
                libc::write(fd, funcname.as_ptr() as *const libc::c_void, funcname.len())
            }
        };
        puts(fd, "\n");
    }

    // faulthandler_dump_traceback
    #[cfg(any(unix, windows))]
    fn faulthandler_dump_traceback(fd: i32, _all_threads: bool) {
        static REENTRANT: AtomicBool = AtomicBool::new(false);

        if REENTRANT.swap(true, Ordering::SeqCst) {
            return;
        }

        // Write thread header
        write_thread_id(fd, true);

        // Try to dump traceback from snapshot
        let count = SNAPSHOT_COUNT.load(Ordering::Acquire);
        if count > 0 {
            // Using index access instead of iterator because FRAME_SNAPSHOTS is static mut
            #[allow(clippy::needless_range_loop)]
            for i in 0..count {
                unsafe {
                    let snap = &FRAME_SNAPSHOTS[i];
                    if snap.filename_len > 0 {
                        dump_frame(
                            fd,
                            &snap.filename[..snap.filename_len],
                            snap.lineno,
                            &snap.funcname[..snap.funcname_len],
                        );
                    }
                }
            }
        } else {
            puts(fd, "  <no Python frame>\n");
        }

        REENTRANT.store(false, Ordering::SeqCst);
    }

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
        // If lasti is 0, execution hasn't started yet - use first line number or 1
        let line = if frame.lasti() == 0 {
            frame.code.first_line_number.map(|n| n.get()).unwrap_or(1)
        } else {
            frame.current_location().line.get()
        };
        format!(
            "  File \"{}\", line {} in {}",
            frame.code.source_path, line, func_name
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

    // faulthandler_py_enable
    #[pyfunction]
    fn enable(args: EnableArgs, vm: &VirtualMachine) -> PyResult<()> {
        // Get file descriptor
        let fd = get_fd_from_file_opt(args.file, vm)?;

        // Store fd and all_threads in global state
        FATAL_ERROR.fd.store(fd, Ordering::Relaxed);
        FATAL_ERROR
            .all_threads
            .store(args.all_threads, Ordering::Relaxed);

        // Install signal handlers
        if !faulthandler_enable_internal() {
            return Err(vm.new_runtime_error("Failed to enable faulthandler".to_owned()));
        }

        Ok(())
    }

    // Signal handlers

    /// faulthandler_disable_fatal_handler (faulthandler.c:310-321)
    #[cfg(unix)]
    unsafe fn faulthandler_disable_fatal_handler(handler: &mut FaultHandler) {
        if !handler.enabled {
            return;
        }
        handler.enabled = false;
        unsafe {
            libc::sigaction(handler.signum, &handler.previous, core::ptr::null_mut());
        }
    }

    #[cfg(windows)]
    unsafe fn faulthandler_disable_fatal_handler(handler: &mut FaultHandler) {
        if !handler.enabled {
            return;
        }
        handler.enabled = false;
        unsafe {
            libc::signal(handler.signum, handler.previous);
        }
    }

    // faulthandler_fatal_error
    #[cfg(unix)]
    extern "C" fn faulthandler_fatal_error(signum: libc::c_int) {
        let save_errno = get_errno();

        if !FATAL_ERROR.enabled.load(Ordering::Relaxed) {
            return;
        }

        let fd = FATAL_ERROR.fd.load(Ordering::Relaxed);

        let handler = unsafe {
            FAULTHANDLER_HANDLERS
                .iter_mut()
                .find(|h| h.signum == signum)
        };

        // faulthandler_fatal_error
        if let Some(h) = handler {
            // Disable handler first (restores previous)
            unsafe {
                faulthandler_disable_fatal_handler(h);
            }

            puts(fd, "Fatal Python error: ");
            puts(fd, h.name);
            puts(fd, "\n\n");
        } else {
            puts(fd, "Fatal Python error from unexpected signum: ");
            dump_decimal(fd, signum as usize);
            puts(fd, "\n\n");
        }

        // faulthandler_dump_traceback
        let all_threads = FATAL_ERROR.all_threads.load(Ordering::Relaxed);
        faulthandler_dump_traceback(fd, all_threads);

        // restore errno
        set_errno(save_errno);

        // raise
        // Called immediately thanks to SA_NODEFER flag
        unsafe {
            libc::raise(signum);
        }
    }

    // faulthandler_fatal_error for Windows
    #[cfg(windows)]
    extern "C" fn faulthandler_fatal_error(signum: libc::c_int) {
        let save_errno = get_errno();

        if !FATAL_ERROR.enabled.load(Ordering::Relaxed) {
            return;
        }

        let fd = FATAL_ERROR.fd.load(Ordering::Relaxed);

        let handler = unsafe {
            FAULTHANDLER_HANDLERS
                .iter_mut()
                .find(|h| h.signum == signum)
        };

        if let Some(h) = handler {
            unsafe {
                faulthandler_disable_fatal_handler(h);
            }
            puts(fd, "Fatal Python error: ");
            puts(fd, h.name);
            puts(fd, "\n\n");
        } else {
            puts(fd, "Fatal Python error from unexpected signum: ");
            dump_decimal(fd, signum as usize);
            puts(fd, "\n\n");
        }

        let all_threads = FATAL_ERROR.all_threads.load(Ordering::Relaxed);
        faulthandler_dump_traceback(fd, all_threads);

        set_errno(save_errno);

        // On Windows, don't explicitly call the previous handler for SIGSEGV
        if signum == libc::SIGSEGV {
            return;
        }

        unsafe {
            libc::raise(signum);
        }
    }

    // faulthandler_enable
    #[cfg(unix)]
    fn faulthandler_enable_internal() -> bool {
        if FATAL_ERROR.enabled.load(Ordering::Relaxed) {
            return true;
        }

        unsafe {
            for handler in FAULTHANDLER_HANDLERS.iter_mut() {
                if handler.enabled {
                    continue;
                }

                let mut action: libc::sigaction = core::mem::zeroed();
                action.sa_sigaction = faulthandler_fatal_error as *const () as libc::sighandler_t;
                // SA_NODEFER flag
                action.sa_flags = libc::SA_NODEFER;

                if libc::sigaction(handler.signum, &action, &mut handler.previous) != 0 {
                    return false;
                }

                handler.enabled = true;
            }
        }

        FATAL_ERROR.enabled.store(true, Ordering::Relaxed);
        true
    }

    #[cfg(windows)]
    fn faulthandler_enable_internal() -> bool {
        if FATAL_ERROR.enabled.load(Ordering::Relaxed) {
            return true;
        }

        unsafe {
            for handler in FAULTHANDLER_HANDLERS.iter_mut() {
                if handler.enabled {
                    continue;
                }

                handler.previous = libc::signal(
                    handler.signum,
                    faulthandler_fatal_error as *const () as libc::sighandler_t,
                );

                // SIG_ERR is -1 as sighandler_t (which is usize on Windows)
                if handler.previous == libc::SIG_ERR as libc::sighandler_t {
                    return false;
                }

                handler.enabled = true;
            }
        }

        FATAL_ERROR.enabled.store(true, Ordering::Relaxed);
        true
    }

    // faulthandler_disable
    #[cfg(any(unix, windows))]
    fn faulthandler_disable_internal() {
        if !FATAL_ERROR.enabled.swap(false, Ordering::Relaxed) {
            return;
        }

        unsafe {
            for handler in FAULTHANDLER_HANDLERS.iter_mut() {
                faulthandler_disable_fatal_handler(handler);
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn faulthandler_enable_internal() -> bool {
        FATAL_ERROR.enabled.store(true, Ordering::Relaxed);
        true
    }

    #[cfg(not(any(unix, windows)))]
    fn faulthandler_disable_internal() {
        FATAL_ERROR.enabled.store(false, Ordering::Relaxed);
    }

    // faulthandler_disable_py
    #[pyfunction]
    fn disable() -> bool {
        let was_enabled = FATAL_ERROR.enabled.load(Ordering::Relaxed);
        faulthandler_disable_internal();
        was_enabled
    }

    // faulthandler_is_enabled
    #[pyfunction]
    fn is_enabled() -> bool {
        FATAL_ERROR.enabled.load(Ordering::Relaxed)
    }

    fn format_timeout(timeout_us: u64) -> String {
        let sec = timeout_us / 1_000_000;
        let us = timeout_us % 1_000_000;
        let min = sec / 60;
        let sec = sec % 60;
        let hour = min / 60;
        let min = min % 60;

        if us != 0 {
            format!("Timeout ({:02}:{:02}:{:02}.{:06})!\n", hour, min, sec, us)
        } else {
            format!("Timeout ({:02}:{:02}:{:02})!\n", hour, min, sec)
        }
    }

    fn get_fd_from_file_opt(file: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<i32> {
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

    fn watchdog_thread(state: WatchdogHandle) {
        let (lock, cvar) = &*state;

        loop {
            // Hold lock across wait_timeout to avoid race condition
            let mut guard = lock.lock();
            if guard.cancel {
                return;
            }
            let timeout = Duration::from_micros(guard.timeout_us);
            cvar.wait_for(&mut guard, timeout);

            // Check if cancelled after wait
            if guard.cancel {
                return;
            }

            // Extract values before releasing lock for I/O
            let (repeat, exit, fd, header) =
                (guard.repeat, guard.exit, guard.fd, guard.header.clone());
            drop(guard); // Release lock before I/O

            // Timeout occurred, dump traceback
            #[cfg(target_arch = "wasm32")]
            let _ = (exit, fd, &header);

            #[cfg(not(target_arch = "wasm32"))]
            {
                let header_bytes = header.as_bytes();
                #[cfg(windows)]
                unsafe {
                    libc::write(
                        fd,
                        header_bytes.as_ptr() as *const libc::c_void,
                        header_bytes.len() as u32,
                    );
                }
                #[cfg(not(windows))]
                unsafe {
                    libc::write(
                        fd,
                        header_bytes.as_ptr() as *const libc::c_void,
                        header_bytes.len(),
                    );
                }

                // Note: We cannot dump actual Python traceback from a separate thread
                // because we don't have access to the VM's frame stack.
                // Just output a message indicating timeout occurred.
                let msg = b"<timeout: cannot dump traceback from watchdog thread>\n";
                #[cfg(windows)]
                unsafe {
                    libc::write(fd, msg.as_ptr() as *const libc::c_void, msg.len() as u32);
                }
                #[cfg(not(windows))]
                unsafe {
                    libc::write(fd, msg.as_ptr() as *const libc::c_void, msg.len());
                }

                if exit {
                    std::process::exit(1);
                }
            }

            if !repeat {
                return;
            }
        }
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct DumpTracebackLaterArgs {
        #[pyarg(positional, error_msg = "timeout must be a number (int or float)")]
        timeout: ArgIntoFloat,
        #[pyarg(any, default = false)]
        repeat: bool,
        #[pyarg(any, default)]
        file: OptionalArg<PyObjectRef>,
        #[pyarg(any, default = false)]
        exit: bool,
    }

    #[pyfunction]
    fn dump_traceback_later(args: DumpTracebackLaterArgs, vm: &VirtualMachine) -> PyResult<()> {
        let timeout: f64 = args.timeout.into_float();

        if timeout <= 0.0 {
            return Err(vm.new_value_error("timeout must be greater than 0".to_owned()));
        }

        let fd = get_fd_from_file_opt(args.file, vm)?;

        // Convert timeout to microseconds
        let timeout_us = (timeout * 1_000_000.0) as u64;
        if timeout_us == 0 {
            return Err(vm.new_value_error("timeout must be greater than 0".to_owned()));
        }

        let header = format_timeout(timeout_us);

        // Cancel any previous watchdog
        cancel_dump_traceback_later();

        // Create new watchdog state
        let state = Arc::new((
            Mutex::new(WatchdogState {
                cancel: false,
                fd,
                timeout_us,
                repeat: args.repeat,
                exit: args.exit,
                header,
            }),
            Condvar::new(),
        ));

        // Store the state
        {
            let mut watchdog = WATCHDOG.lock();
            *watchdog = Some(Arc::clone(&state));
        }

        // Start watchdog thread
        thread::spawn(move || {
            watchdog_thread(state);
        });

        Ok(())
    }

    #[pyfunction]
    fn cancel_dump_traceback_later() {
        let state = {
            let mut watchdog = WATCHDOG.lock();
            watchdog.take()
        };

        if let Some(state) = state {
            let (lock, cvar) = &*state;
            {
                let mut guard = lock.lock();
                guard.cancel = true;
            }
            cvar.notify_all();
        }
    }

    #[cfg(unix)]
    mod user_signals {
        use parking_lot::Mutex;

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
            let guard = USER_SIGNALS.lock();
            guard.as_ref().and_then(|v| v.get(signum).cloned())
        }

        pub fn set_user_signal(signum: usize, signal: UserSignal) {
            let mut guard = USER_SIGNALS.lock();
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
            let mut guard = USER_SIGNALS.lock();
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
            let guard = USER_SIGNALS.lock();
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
                libc::signal(
                    signum,
                    faulthandler_user_signal as *const () as libc::sighandler_t,
                );
            }
        }
    }

    #[cfg(unix)]
    fn check_signum(signum: i32, vm: &VirtualMachine) -> PyResult<()> {
        // Check if it's a fatal signal (faulthandler.c uses faulthandler_handlers array)
        let is_fatal = unsafe { FAULTHANDLER_HANDLERS.iter().any(|h| h.signum == signum) };
        if is_fatal {
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

        let fd = get_fd_from_file_opt(args.file, vm)?;

        let signum = args.signum as usize;

        // Get current handler to save as previous
        let previous = if !user_signals::is_enabled(signum) {
            // Install signal handler
            let prev = unsafe {
                libc::signal(
                    args.signum,
                    faulthandler_user_signal as *const () as libc::sighandler_t,
                )
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
            let ptr: *const i32 = core::ptr::null();
            core::ptr::read_volatile(ptr);
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
            std::thread::sleep(core::time::Duration::from_secs(1));
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
            RaiseException(args.code, args.flags, 0, core::ptr::null());
        }
    }
}
