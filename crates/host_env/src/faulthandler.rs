#![allow(
    clippy::missing_safety_doc,
    reason = "These wrappers expose low-level fault handler hooks with raw OS ABI semantics."
)]
#![allow(
    clippy::result_unit_err,
    reason = "These helpers preserve the existing fault-handler error surface."
)]
#![allow(static_mut_refs)]

#[cfg(unix)]
use alloc::vec::Vec;
#[cfg(unix)]
use parking_lot::Mutex;
#[cfg(windows)]
use windows_sys::Win32::System::{
    Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_POINTERS, PVECTORED_EXCEPTION_HANDLER,
        RaiseException, RemoveVectoredExceptionHandler, SEM_NOGPFAULTERRORBOX, SetErrorMode,
    },
    Threading::GetCurrentThreadId,
};

#[cfg(windows)]
pub type ExceptionPointers = EXCEPTION_POINTERS;

#[cfg(unix)]
struct FatalSignalHandler {
    signum: libc::c_int,
    enabled: bool,
    name: &'static str,
    previous: libc::sigaction,
}

#[cfg(windows)]
struct FatalSignalHandler {
    signum: libc::c_int,
    enabled: bool,
    name: &'static str,
    previous: libc::sighandler_t,
}

#[cfg(unix)]
impl FatalSignalHandler {
    const fn new(signum: libc::c_int, name: &'static str) -> Self {
        Self {
            signum,
            enabled: false,
            name,
            previous: unsafe { core::mem::zeroed() },
        }
    }
}

#[cfg(windows)]
impl FatalSignalHandler {
    const fn new(signum: libc::c_int, name: &'static str) -> Self {
        Self {
            signum,
            enabled: false,
            name,
            previous: 0,
        }
    }
}

#[cfg(unix)]
const FATAL_SIGNAL_COUNT: usize = 5;
#[cfg(windows)]
const FATAL_SIGNAL_COUNT: usize = 4;

#[cfg(unix)]
static mut FATAL_SIGNAL_HANDLERS: [FatalSignalHandler; FATAL_SIGNAL_COUNT] = [
    FatalSignalHandler::new(libc::SIGBUS, "Bus error"),
    FatalSignalHandler::new(libc::SIGILL, "Illegal instruction"),
    FatalSignalHandler::new(libc::SIGFPE, "Floating-point exception"),
    FatalSignalHandler::new(libc::SIGABRT, "Aborted"),
    FatalSignalHandler::new(libc::SIGSEGV, "Segmentation fault"),
];

#[cfg(windows)]
static mut FATAL_SIGNAL_HANDLERS: [FatalSignalHandler; FATAL_SIGNAL_COUNT] = [
    FatalSignalHandler::new(libc::SIGILL, "Illegal instruction"),
    FatalSignalHandler::new(libc::SIGFPE, "Floating-point exception"),
    FatalSignalHandler::new(libc::SIGABRT, "Aborted"),
    FatalSignalHandler::new(libc::SIGSEGV, "Segmentation fault"),
];

#[cfg(unix)]
const USER_SIGNAL_CAPACITY: usize = 64;

#[cfg(unix)]
#[derive(Clone, Copy)]
pub struct UserSignal {
    pub fd: i32,
    pub all_threads: bool,
    pub chain: bool,
}

#[cfg(unix)]
#[derive(Clone, Copy)]
struct RegisteredUserSignal {
    enabled: bool,
    fd: i32,
    all_threads: bool,
    chain: bool,
    previous: libc::sigaction,
}

#[cfg(unix)]
impl Default for RegisteredUserSignal {
    fn default() -> Self {
        Self {
            enabled: false,
            fd: 2,
            all_threads: true,
            chain: false,
            previous: unsafe { core::mem::zeroed() },
        }
    }
}

#[cfg(unix)]
static USER_SIGNALS: Mutex<Option<Vec<RegisteredUserSignal>>> = Mutex::new(None);

pub fn write_fd(fd: i32, buf: &[u8]) {
    let _ = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len() as _) };
}

#[cfg(any(unix, windows))]
pub fn is_fatal_signal(signum: libc::c_int) -> bool {
    unsafe {
        FATAL_SIGNAL_HANDLERS
            .iter()
            .any(|handler| handler.signum == signum)
    }
}

#[cfg(any(unix, windows))]
pub fn fatal_signal_name(signum: libc::c_int) -> Option<&'static str> {
    unsafe {
        FATAL_SIGNAL_HANDLERS
            .iter()
            .find(|handler| handler.signum == signum)
            .map(|handler| handler.name)
    }
}

#[cfg(any(unix, windows))]
pub fn abort_process() -> ! {
    unsafe { libc::abort() }
}

#[cfg(any(unix, windows))]
pub fn raise_signal(signum: libc::c_int) {
    unsafe {
        libc::raise(signum);
    }
}

#[cfg(unix)]
#[inline]
pub fn current_thread_id() -> u64 {
    unsafe { libc::pthread_self() as u64 }
}

#[cfg(windows)]
#[inline]
pub fn current_thread_id() -> u64 {
    unsafe { GetCurrentThreadId() as u64 }
}

#[cfg(unix)]
pub fn install_sigaction(
    signum: libc::c_int,
    handler: extern "C" fn(libc::c_int),
    flags: libc::c_int,
    previous: &mut libc::sigaction,
) -> bool {
    let mut action: libc::sigaction = unsafe { core::mem::zeroed() };
    action.sa_sigaction = handler as *const () as libc::sighandler_t;
    action.sa_flags = flags;
    unsafe { libc::sigaction(signum, &action, previous) == 0 }
}

#[cfg(unix)]
unsafe fn disable_fatal_signal_handler(handler: &mut FatalSignalHandler) {
    if !handler.enabled {
        return;
    }
    handler.enabled = false;
    restore_sigaction(handler.signum, &handler.previous);
}

#[cfg(unix)]
pub fn enable_fatal_handlers(handler: extern "C" fn(libc::c_int), flags: libc::c_int) -> bool {
    unsafe {
        let mut installed = Vec::new();
        for entry in FATAL_SIGNAL_HANDLERS.iter_mut() {
            if entry.enabled {
                continue;
            }

            if !install_sigaction(entry.signum, handler, flags, &mut entry.previous) {
                for signum in installed {
                    disable_fatal_signal(signum);
                }
                return false;
            }
            entry.enabled = true;
            installed.push(entry.signum);
        }
    }
    true
}

#[cfg(unix)]
pub fn disable_fatal_signal(signum: libc::c_int) {
    unsafe {
        if let Some(handler) = FATAL_SIGNAL_HANDLERS
            .iter_mut()
            .find(|handler| handler.signum == signum)
        {
            disable_fatal_signal_handler(handler);
        }
    }
}

#[cfg(unix)]
pub fn disable_fatal_handlers() {
    unsafe {
        for handler in FATAL_SIGNAL_HANDLERS.iter_mut() {
            disable_fatal_signal_handler(handler);
        }
    }
}

#[cfg(unix)]
pub fn restore_sigaction(signum: libc::c_int, previous: &libc::sigaction) {
    unsafe {
        libc::sigaction(signum, previous, core::ptr::null_mut());
    }
}

#[cfg(unix)]
pub fn signal_default_and_raise(signum: libc::c_int) {
    unsafe {
        libc::signal(signum, libc::SIG_DFL);
        libc::raise(signum);
    }
}

#[cfg(unix)]
pub fn exit_immediately(code: libc::c_int) -> ! {
    unsafe { libc::_exit(code) }
}

#[cfg(unix)]
pub fn get_user_signal(signum: usize) -> Option<UserSignal> {
    let guard = USER_SIGNALS.lock();
    guard
        .as_ref()
        .and_then(|signals| signals.get(signum))
        .and_then(|signal| {
            signal.enabled.then_some(UserSignal {
                fd: signal.fd,
                all_threads: signal.all_threads,
                chain: signal.chain,
            })
        })
}

#[cfg(unix)]
pub fn register_user_signal(
    signum: libc::c_int,
    fd: i32,
    all_threads: bool,
    chain: bool,
    handler: extern "C" fn(libc::c_int),
) -> std::io::Result<()> {
    if signum < 0 || signum as usize >= USER_SIGNAL_CAPACITY {
        return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
    }
    let signum = signum as usize;
    let mut guard = USER_SIGNALS.lock();
    if guard.is_none() {
        *guard = Some(vec![RegisteredUserSignal::default(); USER_SIGNAL_CAPACITY]);
    }
    let signals = guard
        .as_mut()
        .expect("user signal table must be initialized");
    let entry = &mut signals[signum];

    if !entry.enabled {
        let mut previous = unsafe { core::mem::zeroed() };
        if !install_sigaction(
            signum as libc::c_int,
            handler,
            if chain {
                libc::SA_NODEFER
            } else {
                libc::SA_RESTART
            },
            &mut previous,
        ) {
            return Err(std::io::Error::last_os_error());
        }
        entry.previous = previous;
    }

    entry.enabled = true;
    entry.fd = fd;
    entry.all_threads = all_threads;
    entry.chain = chain;
    Ok(())
}

#[cfg(unix)]
pub fn unregister_user_signal(signum: libc::c_int) -> bool {
    if signum < 0 {
        return false;
    }
    let signum = signum as usize;
    let mut guard = USER_SIGNALS.lock();
    let Some(signals) = guard.as_mut() else {
        return false;
    };
    let Some(entry) = signals.get_mut(signum) else {
        return false;
    };
    if !entry.enabled {
        return false;
    }

    let previous = entry.previous;
    *entry = RegisteredUserSignal::default();
    restore_sigaction(signum as libc::c_int, &previous);
    true
}

#[cfg(unix)]
pub fn reraise_user_signal(signum: libc::c_int, handler: extern "C" fn(libc::c_int)) -> bool {
    if signum < 0 {
        return false;
    }
    let signum_usize = signum as usize;
    let previous = {
        let guard = USER_SIGNALS.lock();
        let Some(signals) = guard.as_ref() else {
            return false;
        };
        let Some(entry) = signals.get(signum_usize) else {
            return false;
        };
        if !entry.enabled || !entry.chain {
            return false;
        }
        entry.previous
    };

    let saved_errno = crate::os::get_errno();
    restore_sigaction(signum, &previous);
    crate::os::set_errno(saved_errno);
    raise_signal(signum);

    let mut ignored_previous = unsafe { core::mem::zeroed() };
    let _ = install_sigaction(signum, handler, libc::SA_NODEFER, &mut ignored_previous);

    crate::os::set_errno(saved_errno);
    true
}

#[cfg(windows)]
pub fn install_signal_handler(
    signum: libc::c_int,
    handler: extern "C" fn(libc::c_int),
) -> Result<libc::sighandler_t, ()> {
    let previous = unsafe { libc::signal(signum, handler as *const () as libc::sighandler_t) };
    if previous == libc::SIG_ERR as libc::sighandler_t {
        Err(())
    } else {
        Ok(previous)
    }
}

#[cfg(windows)]
unsafe fn disable_fatal_signal_handler(handler: &mut FatalSignalHandler) {
    if !handler.enabled {
        return;
    }
    handler.enabled = false;
    restore_signal_handler(handler.signum, handler.previous);
}

#[cfg(windows)]
pub fn enable_fatal_handlers(handler: extern "C" fn(libc::c_int), _flags: libc::c_int) -> bool {
    unsafe {
        for entry in FATAL_SIGNAL_HANDLERS.iter_mut() {
            if entry.enabled {
                continue;
            }

            let Ok(previous) = install_signal_handler(entry.signum, handler) else {
                return false;
            };
            entry.previous = previous;
            entry.enabled = true;
        }
    }
    true
}

#[cfg(windows)]
pub fn disable_fatal_signal(signum: libc::c_int) {
    unsafe {
        if let Some(handler) = FATAL_SIGNAL_HANDLERS
            .iter_mut()
            .find(|handler| handler.signum == signum)
        {
            disable_fatal_signal_handler(handler);
        }
    }
}

#[cfg(windows)]
pub fn disable_fatal_handlers() {
    unsafe {
        for handler in FATAL_SIGNAL_HANDLERS.iter_mut() {
            disable_fatal_signal_handler(handler);
        }
    }
}

#[cfg(windows)]
pub fn restore_signal_handler(signum: libc::c_int, previous: libc::sighandler_t) {
    unsafe {
        libc::signal(signum, previous);
    }
}

#[cfg(windows)]
pub fn signal_default_and_raise(signum: libc::c_int) {
    unsafe {
        libc::signal(signum, libc::SIG_DFL);
        libc::raise(signum);
    }
}

#[cfg(windows)]
pub fn add_vectored_exception_handler(handler: PVECTORED_EXCEPTION_HANDLER) -> usize {
    unsafe { AddVectoredExceptionHandler(1, handler) as usize }
}

#[cfg(windows)]
pub fn remove_vectored_exception_handler(handle: usize) {
    if handle != 0 {
        unsafe {
            RemoveVectoredExceptionHandler(handle as *mut core::ffi::c_void);
        }
    }
}

#[cfg(windows)]
pub fn suppress_crash_report() {
    unsafe {
        let mode = SetErrorMode(SEM_NOGPFAULTERRORBOX);
        SetErrorMode(mode | SEM_NOGPFAULTERRORBOX);
    }
}

#[cfg(windows)]
pub fn raise_exception(code: u32, flags: u32) {
    unsafe {
        RaiseException(code, flags, 0, core::ptr::null());
    }
}

#[cfg(windows)]
pub fn ignore_exception(code: u32) -> bool {
    if (code & 0x8000_0000) == 0 {
        return true;
    }
    code == 0xE06D7363 || code == 0xE0434352
}

#[cfg(windows)]
pub fn exception_description(code: u32) -> Option<&'static str> {
    match code {
        0xC0000005 => Some("access violation"),
        0xC000008C => Some("float divide by zero"),
        0xC0000091 => Some("float overflow"),
        0xC0000094 => Some("int divide by zero"),
        0xC0000095 => Some("integer overflow"),
        0xC0000006 => Some("page error"),
        0xC00000FD => Some("stack overflow"),
        0xC000001D => Some("illegal instruction"),
        _ => None,
    }
}

#[cfg(windows)]
pub unsafe fn exception_code(exc_info: *mut EXCEPTION_POINTERS) -> u32 {
    let record = unsafe { &*(*exc_info).ExceptionRecord };
    record.ExceptionCode as u32
}

#[cfg(windows)]
#[inline]
pub fn is_access_violation(code: u32) -> bool {
    code == 0xC0000005
}
