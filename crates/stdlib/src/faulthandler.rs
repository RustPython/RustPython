pub(crate) use decl::module_def;

#[allow(static_mut_refs)] // TODO: group code only with static mut refs
#[pymodule(name = "faulthandler")]
mod decl {
    use crate::vm::{
        PyObjectRef, PyResult, VirtualMachine,
        frame::Frame,
        function::{ArgIntoFloat, OptionalArg},
    };
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use core::time::Duration;
    use parking_lot::{Condvar, Mutex};
    #[cfg(any(unix, windows))]
    use rustpython_host_env::faulthandler as host_faulthandler;
    #[cfg(any(unix, windows))]
    use rustpython_host_env::os::{get_errno, set_errno};
    use std::thread;

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

    #[cfg(feature = "threading")]
    type ThreadFrameSlot = Arc<rustpython_vm::vm::thread::ThreadSlot>;

    // Watchdog thread state for dump_traceback_later
    struct WatchdogState {
        cancel: bool,
        fd: i32,
        timeout_us: u64,
        repeat: bool,
        exit: bool,
        header: String,
        #[cfg(feature = "threading")]
        thread_frame_slots: Vec<(u64, ThreadFrameSlot)>,
    }

    type WatchdogHandle = Arc<(Mutex<WatchdogState>, Condvar)>;
    static WATCHDOG: Mutex<Option<WatchdogHandle>> = Mutex::new(None);

    // Signal-safe output functions

    // PUTS macro
    #[cfg(any(unix, windows))]
    fn puts(fd: i32, s: &str) {
        host_faulthandler::write_fd(fd, s.as_bytes());
    }

    #[cfg(any(unix, windows))]
    fn puts_bytes(fd: i32, s: &[u8]) {
        host_faulthandler::write_fd(fd, s);
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

        host_faulthandler::write_fd(fd, &buf[..2 + width]);
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

        host_faulthandler::write_fd(fd, &buf[i..]);
    }

    /// Get current thread ID
    #[cfg(any(unix, windows))]
    fn current_thread_id() -> u64 {
        host_faulthandler::current_thread_id()
    }

    // write_thread_id (traceback.c:1240-1256)
    #[cfg(any(unix, windows))]
    fn write_thread_id(fd: i32, thread_id: u64, is_current: bool) {
        if is_current {
            puts(fd, "Current thread ");
        } else {
            puts(fd, "Thread ");
        }
        dump_hexadecimal(fd, thread_id, core::mem::size_of::<usize>() * 2);
        puts(fd, " (most recent call first):\n");
    }

    /// Dump the current thread's live frame chain to fd (signal-safe).
    /// Walks the `Frame.previous` pointer chain starting from the
    /// thread-local current frame pointer.
    #[cfg(any(unix, windows))]
    fn dump_live_frames(fd: i32) {
        const MAX_FRAME_DEPTH: usize = 100;

        let mut frame_ptr = crate::vm::vm::thread::get_current_frame();
        if frame_ptr.is_null() {
            puts(fd, "  <no Python frame>\n");
            return;
        }
        let mut depth = 0;
        while !frame_ptr.is_null() && depth < MAX_FRAME_DEPTH {
            let frame = unsafe { &*frame_ptr };
            dump_frame_from_raw(fd, frame);
            frame_ptr = frame.previous_frame();
            depth += 1;
        }
        if depth >= MAX_FRAME_DEPTH && !frame_ptr.is_null() {
            puts(fd, "  ...\n");
        }
    }

    /// Dump a single frame's info to fd (signal-safe), reading live data.
    #[cfg(any(unix, windows))]
    fn dump_frame_from_raw(fd: i32, frame: &Frame) {
        let filename = frame.code.source_path().as_str();
        let funcname = frame.code.obj_name.as_str();
        let lasti = frame.lasti();
        let lineno = if lasti == 0 {
            frame.code.first_line_number.map(|n| n.get()).unwrap_or(1) as u32
        } else {
            let idx = (lasti as usize).saturating_sub(1);
            if idx < frame.code.locations.len() {
                frame.code.locations[idx].0.line.get() as u32
            } else {
                frame.code.first_line_number.map(|n| n.get()).unwrap_or(0) as u32
            }
        };

        puts(fd, "  File \"");
        dump_ascii(fd, filename);
        puts(fd, "\", line ");
        dump_decimal(fd, lineno as usize);
        puts(fd, " in ");
        dump_ascii(fd, funcname);
        puts(fd, "\n");
    }

    // faulthandler_dump_traceback (signal-safe, for fatal errors)
    #[cfg(any(unix, windows))]
    fn faulthandler_dump_traceback(fd: i32, all_threads: bool) {
        static REENTRANT: AtomicBool = AtomicBool::new(false);

        if REENTRANT.swap(true, Ordering::SeqCst) {
            return;
        }

        // Write thread header
        if all_threads {
            write_thread_id(fd, current_thread_id(), true);
        } else {
            puts(fd, "Stack (most recent call first):\n");
        }

        dump_live_frames(fd);

        REENTRANT.store(false, Ordering::SeqCst);
    }

    /// MAX_STRING_LENGTH in traceback.c
    const MAX_STRING_LENGTH: usize = 500;

    /// Truncate a UTF-8 string to at most `max_bytes` without splitting a
    /// multi-byte codepoint. Signal-safe (no allocation, no panic).
    #[cfg(any(unix, windows))]
    fn safe_truncate(s: &str, max_bytes: usize) -> (&str, bool) {
        if s.len() <= max_bytes {
            return (s, false);
        }
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        (&s[..end], true)
    }

    /// Write a string to fd, truncating with "..." if it exceeds MAX_STRING_LENGTH.
    /// Mirrors `_Py_DumpASCII` truncation behavior.
    #[cfg(any(unix, windows))]
    fn dump_ascii(fd: i32, s: &str) {
        let (truncated_s, was_truncated) = safe_truncate(s, MAX_STRING_LENGTH);
        puts(fd, truncated_s);
        if was_truncated {
            puts(fd, "...");
        }
    }

    /// Write a frame's info to an fd using signal-safe I/O.
    #[cfg(any(unix, windows))]
    fn dump_frame_from_ref(fd: i32, frame: &crate::vm::Py<Frame>) {
        let funcname = frame.code.obj_name.as_str();
        let filename = frame.code.source_path().as_str();
        let lineno = if frame.lasti() == 0 {
            frame.code.first_line_number.map(|n| n.get()).unwrap_or(1) as u32
        } else {
            frame.current_location().line.get() as u32
        };

        puts(fd, "  File \"");
        dump_ascii(fd, filename);
        puts(fd, "\", line ");
        dump_decimal(fd, lineno as usize);
        puts(fd, " in ");
        dump_ascii(fd, funcname);
        puts(fd, "\n");
    }

    /// Dump traceback for a thread given its frame stack (for cross-thread dumping).
    /// # Safety
    /// Each `FramePtr` must point to a live frame (caller holds the Mutex).
    #[cfg(all(any(unix, windows), feature = "threading"))]
    fn dump_traceback_thread_frames(
        fd: i32,
        thread_id: u64,
        is_current: bool,
        frames: &[rustpython_vm::vm::FramePtr],
    ) {
        write_thread_id(fd, thread_id, is_current);

        if frames.is_empty() {
            puts(fd, "  <no Python frame>\n");
        } else {
            for fp in frames.iter().rev() {
                // SAFETY: caller holds the Mutex, so the owning thread can't pop.
                dump_frame_from_ref(fd, unsafe { fp.as_ref() });
            }
        }
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
        let fd = get_fd_from_file_opt(args.file, vm)?;

        #[cfg(any(unix, windows))]
        {
            if args.all_threads {
                dump_all_threads(fd, vm);
            } else {
                puts(fd, "Stack (most recent call first):\n");
                let frames = vm.frames.borrow();
                for fp in frames.iter().rev() {
                    // SAFETY: the frame is alive while it's in the Vec
                    dump_frame_from_ref(fd, unsafe { fp.as_ref() });
                }
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = (fd, args.all_threads);
        }

        Ok(())
    }

    /// Dump tracebacks of all threads.
    #[cfg(any(unix, windows))]
    fn dump_all_threads(fd: i32, vm: &VirtualMachine) {
        // Get all threads' frame stacks from the shared registry
        #[cfg(feature = "threading")]
        {
            let current_tid = rustpython_vm::stdlib::_thread::get_ident();
            let registry = vm.state.thread_frames.lock();

            // First dump non-current threads, then current thread last
            for (&tid, slot) in registry.iter() {
                if tid == current_tid {
                    continue;
                }
                let frames_guard = slot.frames.lock();
                dump_traceback_thread_frames(fd, tid, false, &frames_guard);
                puts(fd, "\n");
            }

            // Now dump current thread (use vm.frames for most up-to-date data)
            write_thread_id(fd, current_tid, true);
            let frames = vm.frames.borrow();
            if frames.is_empty() {
                puts(fd, "  <no Python frame>\n");
            } else {
                for fp in frames.iter().rev() {
                    dump_frame_from_ref(fd, unsafe { fp.as_ref() });
                }
            }
        }

        #[cfg(not(feature = "threading"))]
        {
            write_thread_id(fd, current_thread_id(), true);
            let frames = vm.frames.borrow();
            for fp in frames.iter().rev() {
                dump_frame_from_ref(fd, unsafe { fp.as_ref() });
            }
        }
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
            return Err(vm.new_runtime_error("Failed to enable faulthandler"));
        }

        Ok(())
    }

    // Signal handlers

    // faulthandler_fatal_error
    #[cfg(unix)]
    extern "C" fn faulthandler_fatal_error(signum: libc::c_int) {
        let save_errno = get_errno();

        if !FATAL_ERROR.enabled.load(Ordering::Relaxed) {
            return;
        }

        let fd = FATAL_ERROR.fd.load(Ordering::Relaxed);

        if let Some(name) = host_faulthandler::fatal_signal_name(signum) {
            host_faulthandler::disable_fatal_signal(signum);
            puts(fd, "Fatal Python error: ");
            puts(fd, name);
            puts(fd, "\n\n");
        } else {
            puts(fd, "Fatal Python error from unexpected signum: ");
            dump_decimal(fd, signum as usize);
            puts(fd, "\n\n");
        }

        let all_threads = FATAL_ERROR.all_threads.load(Ordering::Relaxed);
        faulthandler_dump_traceback(fd, all_threads);

        set_errno(save_errno);

        // Reset to default handler and re-raise to ensure process terminates.
        // We cannot just restore the previous handler because Rust's runtime
        // may have installed its own SIGSEGV handler (for stack overflow detection)
        // that doesn't terminate the process on software-raised signals.
        host_faulthandler::signal_default_and_raise(signum);

        // Fallback if raise() somehow didn't terminate the process
        host_faulthandler::exit_immediately(1);
    }

    // faulthandler_fatal_error for Windows
    #[cfg(windows)]
    extern "C" fn faulthandler_fatal_error(signum: libc::c_int) {
        let save_errno = get_errno();

        if !FATAL_ERROR.enabled.load(Ordering::Relaxed) {
            return;
        }

        let fd = FATAL_ERROR.fd.load(Ordering::Relaxed);

        if let Some(name) = host_faulthandler::fatal_signal_name(signum) {
            host_faulthandler::disable_fatal_signal(signum);
            puts(fd, "Fatal Python error: ");
            puts(fd, name);
            puts(fd, "\n\n");
        } else {
            puts(fd, "Fatal Python error from unexpected signum: ");
            dump_decimal(fd, signum as usize);
            puts(fd, "\n\n");
        }

        let all_threads = FATAL_ERROR.all_threads.load(Ordering::Relaxed);
        faulthandler_dump_traceback(fd, all_threads);

        set_errno(save_errno);

        host_faulthandler::signal_default_and_raise(signum);

        // Fallback
        rustpython_host_env::os::exit(1);
    }

    // Windows vectored exception handler (faulthandler.c:417-480)
    #[cfg(windows)]
    static EXC_HANDLER: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

    #[cfg(windows)]
    fn faulthandler_ignore_exception(code: u32) -> bool {
        host_faulthandler::ignore_exception(code)
    }

    #[cfg(windows)]
    unsafe extern "system" fn faulthandler_exc_handler(
        exc_info: *mut host_faulthandler::ExceptionPointers,
    ) -> i32 {
        const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

        if !FATAL_ERROR.enabled.load(Ordering::Relaxed) {
            return EXCEPTION_CONTINUE_SEARCH;
        }

        let code = unsafe { host_faulthandler::exception_code(exc_info) };

        if faulthandler_ignore_exception(code) {
            return EXCEPTION_CONTINUE_SEARCH;
        }

        let fd = FATAL_ERROR.fd.load(Ordering::Relaxed);

        puts(fd, "Windows fatal exception: ");
        if let Some(description) = host_faulthandler::exception_description(code) {
            puts(fd, description);
        } else {
            puts(fd, "code ");
            dump_hexadecimal(fd, code as u64, 8);
        }
        puts(fd, "\n\n");

        // Disable SIGSEGV handler for access violations to avoid double output
        if host_faulthandler::is_access_violation(code) {
            host_faulthandler::disable_fatal_signal(libc::SIGSEGV);
        }

        let all_threads = FATAL_ERROR.all_threads.load(Ordering::Relaxed);
        faulthandler_dump_traceback(fd, all_threads);

        EXCEPTION_CONTINUE_SEARCH
    }

    // faulthandler_enable
    #[cfg(unix)]
    fn faulthandler_enable_internal() -> bool {
        if FATAL_ERROR.enabled.load(Ordering::Relaxed) {
            return true;
        }

        if !host_faulthandler::enable_fatal_handlers(faulthandler_fatal_error, libc::SA_NODEFER) {
            return false;
        }

        FATAL_ERROR.enabled.store(true, Ordering::Relaxed);
        true
    }

    #[cfg(windows)]
    fn faulthandler_enable_internal() -> bool {
        if FATAL_ERROR.enabled.load(Ordering::Relaxed) {
            return true;
        }

        if !host_faulthandler::enable_fatal_handlers(faulthandler_fatal_error, 0) {
            return false;
        }

        // Register Windows vectored exception handler
        #[cfg(windows)]
        {
            let h =
                host_faulthandler::add_vectored_exception_handler(Some(faulthandler_exc_handler));
            EXC_HANDLER.store(h as usize, Ordering::Relaxed);
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

        host_faulthandler::disable_fatal_handlers();

        // Remove Windows vectored exception handler
        #[cfg(windows)]
        {
            let h = EXC_HANDLER.swap(0, Ordering::Relaxed);
            host_faulthandler::remove_vectored_exception_handler(h);
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

        // Match Python's timedelta str format: H:MM:SS.ffffff (no leading zero for hours)
        if us != 0 {
            format!("Timeout ({}:{:02}:{:02}.{:06})!\n", hour, min, sec, us)
        } else {
            format!("Timeout ({}:{:02}:{:02})!\n", hour, min, sec)
        }
    }

    fn get_fd_from_file_opt(file: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<i32> {
        match file {
            OptionalArg::Present(f) if !vm.is_none(&f) => {
                // Check if it's an integer (file descriptor)
                if let Ok(fd) = f.try_to_value::<i32>(vm) {
                    if fd < 0 {
                        return Err(vm.new_value_error("file is not a valid file descriptor"));
                    }
                    return Ok(fd);
                }
                // Try to get fileno() from file object
                let fileno = vm.call_method(&f, "fileno", ())?;
                let fd: i32 = fileno.try_to_value(vm)?;
                if fd < 0 {
                    return Err(vm.new_value_error("file is not a valid file descriptor"));
                }
                // Try to flush the file
                let _ = vm.call_method(&f, "flush", ());
                Ok(fd)
            }
            _ => {
                // file=None or file not passed: fall back to sys.stderr
                let stderr = vm.sys_module.get_attr("stderr", vm)?;
                if vm.is_none(&stderr) {
                    return Err(vm.new_runtime_error("sys.stderr is None"));
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
            let repeat = guard.repeat;
            let exit = guard.exit;
            let fd = guard.fd;
            let header = guard.header.clone();
            #[cfg(feature = "threading")]
            let thread_frame_slots = guard.thread_frame_slots.clone();
            drop(guard); // Release lock before I/O

            // Timeout occurred, dump traceback
            #[cfg(target_arch = "wasm32")]
            let _ = (exit, fd, &header);

            #[cfg(not(target_arch = "wasm32"))]
            {
                puts_bytes(fd, header.as_bytes());

                // Use thread frame slots when threading is enabled (includes all threads).
                // Fall back to live frame walking for non-threaded builds.
                #[cfg(feature = "threading")]
                {
                    for (tid, slot) in &thread_frame_slots {
                        let frames = slot.frames.lock();
                        dump_traceback_thread_frames(fd, *tid, false, &frames);
                    }
                }
                #[cfg(not(feature = "threading"))]
                {
                    write_thread_id(fd, current_thread_id(), false);
                    dump_live_frames(fd);
                }

                if exit {
                    rustpython_host_env::os::exit(1);
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
            return Err(vm.new_value_error("timeout must be greater than 0"));
        }

        let fd = get_fd_from_file_opt(args.file, vm)?;

        // Convert timeout to microseconds
        let timeout_us = (timeout * 1_000_000.0) as u64;
        if timeout_us == 0 {
            return Err(vm.new_value_error("timeout must be greater than 0"));
        }

        let header = format_timeout(timeout_us);

        // Snapshot thread frame slots so watchdog can dump tracebacks
        #[cfg(feature = "threading")]
        let thread_frame_slots: Vec<(u64, ThreadFrameSlot)> = {
            let registry = vm.state.thread_frames.lock();
            registry
                .iter()
                .map(|(&id, slot)| (id, Arc::clone(slot)))
                .collect()
        };

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
                #[cfg(feature = "threading")]
                thread_frame_slots,
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
    extern "C" fn faulthandler_user_signal(signum: libc::c_int) {
        let save_errno = get_errno();

        let user = match host_faulthandler::get_user_signal(signum as usize) {
            Some(u) => u,
            _ => return,
        };

        faulthandler_dump_traceback(user.fd, user.all_threads);

        if user.chain {
            set_errno(save_errno);
            let _ = host_faulthandler::reraise_user_signal(signum, faulthandler_user_signal);
        }
    }

    #[cfg(unix)]
    fn check_signum(signum: i32, vm: &VirtualMachine) -> PyResult<()> {
        // Check if it's a fatal signal (faulthandler.c uses faulthandler_handlers array)
        if host_faulthandler::is_fatal_signal(signum) {
            return Err(vm.new_runtime_error(format!(
                "signal {} cannot be registered, use enable() instead",
                signum
            )));
        }

        // Check if signal is in valid range
        if !(1..64).contains(&signum) {
            return Err(vm.new_value_error("signal number out of range"));
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

        host_faulthandler::register_user_signal(
            args.signum,
            fd,
            args.all_threads,
            args.chain,
            faulthandler_user_signal,
        )
        .map_err(|_| {
            vm.new_os_error(format!(
                "Failed to register signal handler for signal {}",
                args.signum
            ))
        })?;

        Ok(())
    }

    #[cfg(unix)]
    #[pyfunction]
    fn unregister(signum: i32, vm: &VirtualMachine) -> PyResult<bool> {
        check_signum(signum, vm)?;
        Ok(host_faulthandler::unregister_user_signal(signum))
    }

    // Test functions for faulthandler testing

    #[pyfunction]
    fn _read_null(_vm: &VirtualMachine) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            suppress_crash_report();

            unsafe {
                let ptr: *const i32 = core::ptr::null();
                core::ptr::read_volatile(ptr);
            }
        }
    }

    #[derive(FromArgs)]
    #[allow(dead_code)]
    struct SigsegvArgs {
        #[pyarg(any, default = false)]
        release_gil: bool,
    }

    #[pyfunction]
    fn _sigsegv(_args: SigsegvArgs, _vm: &VirtualMachine) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            suppress_crash_report();

            // Write to NULL pointer to trigger a real hardware SIGSEGV,
            // matching CPython's *((volatile int *)NULL) = 0;
            // Using raise(SIGSEGV) doesn't work reliably because Rust's runtime
            // installs its own signal handler that may swallow software signals.
            unsafe {
                let ptr: *mut i32 = core::ptr::null_mut();
                core::ptr::write_volatile(ptr, 0);
            }
        }
    }

    #[pyfunction]
    fn _sigabrt(_vm: &VirtualMachine) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            suppress_crash_report();
            host_faulthandler::abort_process();
        }
    }

    #[pyfunction]
    fn _sigfpe(_vm: &VirtualMachine) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            suppress_crash_report();
            host_faulthandler::raise_signal(libc::SIGFPE);
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
            host_faulthandler::suppress_crash_report();
        }

        #[cfg(unix)]
        {
            #[cfg(not(any(target_os = "redox", target_os = "wasi")))]
            {
                rustpython_host_env::resource::disable_core_dumps();
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
    fn _raise_exception(args: RaiseExceptionArgs, _vm: &VirtualMachine) {
        suppress_crash_report();
        host_faulthandler::raise_exception(args.code, args.flags);
    }
}
