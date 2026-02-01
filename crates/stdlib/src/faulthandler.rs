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

    // Signal handlers use mutable statics matching faulthandler.c implementation.
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

    /// Arc<Mutex<Vec<FrameRef>>> - shared frame slot for a thread
    #[cfg(feature = "threading")]
    type ThreadFrameSlot = Arc<parking_lot::Mutex<Vec<crate::vm::frame::FrameRef>>>;

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

    #[cfg(any(unix, windows))]
    fn puts_bytes(fd: i32, s: &[u8]) {
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
        let filename = frame.code.source_path.as_str();
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
    fn dump_frame_from_ref(fd: i32, frame: &crate::vm::PyRef<Frame>) {
        let funcname = frame.code.obj_name.as_str();
        let filename = frame.code.source_path.as_str();
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
    #[cfg(all(any(unix, windows), feature = "threading"))]
    fn dump_traceback_thread_frames(
        fd: i32,
        thread_id: u64,
        is_current: bool,
        frames: &[crate::vm::frame::FrameRef],
    ) {
        write_thread_id(fd, thread_id, is_current);

        if frames.is_empty() {
            puts(fd, "  <no Python frame>\n");
        } else {
            for frame in frames.iter().rev() {
                dump_frame_from_ref(fd, frame);
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
                for frame in frames.iter().rev() {
                    dump_frame_from_ref(fd, frame);
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
            let current_tid = rustpython_vm::stdlib::thread::get_ident();
            let registry = vm.state.thread_frames.lock();

            // First dump non-current threads, then current thread last
            for (&tid, slot) in registry.iter() {
                if tid == current_tid {
                    continue;
                }
                let frames_guard = slot.lock();
                dump_traceback_thread_frames(fd, tid, false, &frames_guard);
                puts(fd, "\n");
            }

            // Now dump current thread (use vm.frames for most up-to-date data)
            write_thread_id(fd, current_tid, true);
            let frames = vm.frames.borrow();
            if frames.is_empty() {
                puts(fd, "  <no Python frame>\n");
            } else {
                for frame in frames.iter().rev() {
                    dump_frame_from_ref(fd, frame);
                }
            }
        }

        #[cfg(not(feature = "threading"))]
        {
            write_thread_id(fd, current_thread_id(), true);
            let frames = vm.frames.borrow();
            for frame in frames.iter().rev() {
                dump_frame_from_ref(fd, frame);
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

        if let Some(h) = handler {
            // Disable handler (restores previous)
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

        // Reset to default handler and re-raise to ensure process terminates.
        // We cannot just restore the previous handler because Rust's runtime
        // may have installed its own SIGSEGV handler (for stack overflow detection)
        // that doesn't terminate the process on software-raised signals.
        unsafe {
            libc::signal(signum, libc::SIG_DFL);
            libc::raise(signum);
        }

        // Fallback if raise() somehow didn't terminate the process
        unsafe {
            libc::_exit(1);
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

        unsafe {
            libc::signal(signum, libc::SIG_DFL);
            libc::raise(signum);
        }

        // Fallback
        std::process::exit(1);
    }

    // Windows vectored exception handler (faulthandler.c:417-480)
    #[cfg(windows)]
    static EXC_HANDLER: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

    #[cfg(windows)]
    fn faulthandler_ignore_exception(code: u32) -> bool {
        // bpo-30557: ignore exceptions which are not errors
        if (code & 0x80000000) == 0 {
            return true;
        }
        // bpo-31701: ignore MSC and COM exceptions
        if code == 0xE06D7363 || code == 0xE0434352 {
            return true;
        }
        false
    }

    #[cfg(windows)]
    unsafe extern "system" fn faulthandler_exc_handler(
        exc_info: *mut windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
    ) -> i32 {
        const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

        if !FATAL_ERROR.enabled.load(Ordering::Relaxed) {
            return EXCEPTION_CONTINUE_SEARCH;
        }

        let record = unsafe { &*(*exc_info).ExceptionRecord };
        let code = record.ExceptionCode as u32;

        if faulthandler_ignore_exception(code) {
            return EXCEPTION_CONTINUE_SEARCH;
        }

        let fd = FATAL_ERROR.fd.load(Ordering::Relaxed);

        puts(fd, "Windows fatal exception: ");
        match code {
            0xC0000005 => puts(fd, "access violation"),
            0xC000008C => puts(fd, "float divide by zero"),
            0xC0000091 => puts(fd, "float overflow"),
            0xC0000094 => puts(fd, "int divide by zero"),
            0xC0000095 => puts(fd, "integer overflow"),
            0xC0000006 => puts(fd, "page error"),
            0xC00000FD => puts(fd, "stack overflow"),
            0xC000001D => puts(fd, "illegal instruction"),
            _ => {
                puts(fd, "code ");
                dump_hexadecimal(fd, code as u64, 8);
            }
        }
        puts(fd, "\n\n");

        // Disable SIGSEGV handler for access violations to avoid double output
        if code == 0xC0000005 {
            unsafe {
                for handler in FAULTHANDLER_HANDLERS.iter_mut() {
                    if handler.signum == libc::SIGSEGV {
                        faulthandler_disable_fatal_handler(handler);
                        break;
                    }
                }
            }
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

        // Register Windows vectored exception handler
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Diagnostics::Debug::AddVectoredExceptionHandler;
            let h = unsafe { AddVectoredExceptionHandler(1, Some(faulthandler_exc_handler)) };
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

        unsafe {
            for handler in FAULTHANDLER_HANDLERS.iter_mut() {
                faulthandler_disable_fatal_handler(handler);
            }
        }

        // Remove Windows vectored exception handler
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Diagnostics::Debug::RemoveVectoredExceptionHandler;
            let h = EXC_HANDLER.swap(0, Ordering::Relaxed);
            if h != 0 {
                unsafe {
                    RemoveVectoredExceptionHandler(h as *mut core::ffi::c_void);
                }
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
            _ => {
                // file=None or file not passed: fall back to sys.stderr
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
                        let frames = slot.lock();
                        dump_traceback_thread_frames(fd, *tid, false, &frames);
                    }
                }
                #[cfg(not(feature = "threading"))]
                {
                    write_thread_id(fd, current_thread_id(), false);
                    dump_live_frames(fd);
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
    mod user_signals {
        use parking_lot::Mutex;

        const NSIG: usize = 64;

        #[derive(Clone, Copy)]
        pub struct UserSignal {
            pub enabled: bool,
            pub fd: i32,
            pub all_threads: bool,
            pub chain: bool,
            pub previous: libc::sigaction,
        }

        impl Default for UserSignal {
            fn default() -> Self {
                Self {
                    enabled: false,
                    fd: 2, // stderr
                    all_threads: true,
                    chain: false,
                    // SAFETY: sigaction is a C struct that can be zero-initialized
                    previous: unsafe { core::mem::zeroed() },
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
                let old = v[signum];
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
        let save_errno = get_errno();

        let user = match user_signals::get_user_signal(signum as usize) {
            Some(u) if u.enabled => u,
            _ => return,
        };

        faulthandler_dump_traceback(user.fd, user.all_threads);

        if user.chain {
            // Restore the previous handler and re-raise
            unsafe {
                libc::sigaction(signum, &user.previous, core::ptr::null_mut());
            }
            set_errno(save_errno);
            unsafe {
                libc::raise(signum);
            }
            // Re-install our handler with the same flags as register()
            let save_errno2 = get_errno();
            unsafe {
                let mut action: libc::sigaction = core::mem::zeroed();
                action.sa_sigaction = faulthandler_user_signal as *const () as libc::sighandler_t;
                action.sa_flags = libc::SA_NODEFER;
                libc::sigaction(signum, &action, core::ptr::null_mut());
            }
            set_errno(save_errno2);
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
            unsafe {
                let mut action: libc::sigaction = core::mem::zeroed();
                action.sa_sigaction = faulthandler_user_signal as *const () as libc::sighandler_t;
                // SA_RESTART by default; SA_NODEFER only when chaining
                // (faulthandler.c:860-864)
                action.sa_flags = if args.chain {
                    libc::SA_NODEFER
                } else {
                    libc::SA_RESTART
                };

                let mut prev: libc::sigaction = core::mem::zeroed();
                if libc::sigaction(args.signum, &action, &mut prev) != 0 {
                    return Err(vm.new_os_error(format!(
                        "Failed to register signal handler for signal {}",
                        args.signum
                    )));
                }
                prev
            }
        } else {
            // Already registered, keep previous handler
            user_signals::get_user_signal(signum)
                .map(|u| u.previous)
                .unwrap_or(unsafe { core::mem::zeroed() })
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
                libc::sigaction(signum, &old.previous, core::ptr::null_mut());
            }
            Ok(true)
        } else {
            Ok(false)
        }
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

            unsafe {
                libc::abort();
            }
        }
    }

    #[pyfunction]
    fn _sigfpe(_vm: &VirtualMachine) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            suppress_crash_report();

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
    fn _raise_exception(args: RaiseExceptionArgs, _vm: &VirtualMachine) {
        use windows_sys::Win32::System::Diagnostics::Debug::RaiseException;

        suppress_crash_report();
        unsafe {
            RaiseException(args.code, args.flags, 0, core::ptr::null());
        }
    }
}
