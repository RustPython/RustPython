#![allow(
    clippy::missing_safety_doc,
    reason = "These wrappers expose low-level fault handler hooks with raw OS ABI semantics."
)]
#![allow(
    clippy::result_unit_err,
    reason = "These helpers preserve the existing fault-handler error surface."
)]

#[cfg(windows)]
use windows_sys::Win32::System::{
    Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_POINTERS, PVECTORED_EXCEPTION_HANDLER,
        RaiseException, RemoveVectoredExceptionHandler, SEM_NOGPFAULTERRORBOX, SetErrorMode,
    },
    Threading::GetCurrentThreadId,
};

pub fn write_fd(fd: i32, buf: &[u8]) {
    let _ = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len() as _) };
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
