#[cfg(any(unix, windows))]
use std::io;
#[cfg(windows)]
use std::sync::Once;

#[cfg(unix)]
use crate::os::CheckLibcResult;
#[cfg(any(unix, windows))]
use crate::os::CheckLibcZero;

#[cfg(any(unix, windows))]
pub use libc::sighandler_t;

#[cfg(unix)]
pub use libc::{SIG_DFL, SIG_ERR, SIG_IGN};

#[cfg(not(any(unix, target_os = "wasi")))]
pub const SIG_DFL: libc::sighandler_t = 0;
#[cfg(not(any(unix, target_os = "wasi")))]
pub const SIG_IGN: libc::sighandler_t = 1;
#[cfg(not(any(unix, target_os = "wasi")))]
pub const SIG_ERR: libc::sighandler_t = -1 as _;

/// wasi-libc's userspace emulation of `signal()`/`raise()`, linked from
/// `libwasi-emulated-signal` (see `crates/host_env/vendor/wasm32-wasip1`).
/// WebAssembly has no asynchronous signal delivery, so this is a
/// synchronous, in-process handler table: a handler only runs when this
/// process's own code calls `raise()`, never from an outside interrupt.
/// Numbering matches wasi-libc's `bits/signal.h`, the same values CPython's
/// WASI build gets by linking the same library.
#[cfg(target_os = "wasi")]
mod wasm {
    use std::io;

    #[allow(non_camel_case_types)]
    pub type sighandler_t = usize;

    pub const SIG_DFL: sighandler_t = 0;
    pub const SIG_IGN: sighandler_t = 1;
    pub const SIG_ERR: sighandler_t = -1isize as usize;

    pub const SIGINT: i32 = 2;
    pub const SIGILL: i32 = 4;
    pub const SIGABRT: i32 = 6;
    pub const SIGFPE: i32 = 8;
    pub const SIGSEGV: i32 = 11;
    pub const SIGTERM: i32 = 15;

    unsafe extern "C" {
        fn signal(signum: i32, handler: sighandler_t) -> sighandler_t;
        fn raise(signum: i32) -> i32;
    }

    /// # Safety
    ///
    /// The caller must ensure `signalnum` is a valid platform signal number.
    pub unsafe fn probe_handler(signalnum: i32) -> Option<sighandler_t> {
        let handler = unsafe { signal(signalnum, SIG_IGN) };
        if handler == SIG_ERR {
            None
        } else {
            unsafe { signal(signalnum, handler) };
            Some(handler)
        }
    }

    /// # Safety
    ///
    /// The caller must ensure `signalnum` is a valid platform signal number and
    /// `handler` is accepted by the platform signal ABI.
    pub unsafe fn install_handler(
        signalnum: i32,
        handler: sighandler_t,
    ) -> io::Result<sighandler_t> {
        let old = unsafe { signal(signalnum, handler) };
        if old == SIG_ERR {
            return Err(io::Error::last_os_error());
        }
        Ok(old)
    }

    pub fn raise_signal(signalnum: i32) -> io::Result<()> {
        if unsafe { raise(signalnum) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(target_os = "wasi")]
pub use wasm::*;

#[cfg(unix)]
pub use libc::{SIG_BLOCK, SIG_SETMASK, SIG_UNBLOCK};

#[cfg(any(unix, windows))]
pub use libc::{SIGABRT, SIGFPE, SIGILL, SIGINT, SIGSEGV, SIGTERM};

#[cfg(unix)]
pub use libc::{
    SIGALRM, SIGBUS, SIGCHLD, SIGCONT, SIGHUP, SIGIO, SIGKILL, SIGPIPE, SIGPROF, SIGQUIT, SIGSTOP,
    SIGSYS, SIGTRAP, SIGTSTP, SIGTTIN, SIGTTOU, SIGURG, SIGUSR1, SIGUSR2, SIGVTALRM, SIGWINCH,
    SIGXCPU, SIGXFSZ,
};

#[cfg(all(
    unix,
    not(any(
        target_vendor = "apple",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd"
    ))
))]
pub use libc::{SIGPWR, SIGSTKFLT};

#[cfg(all(unix, not(target_os = "android")))]
pub use libc::{ITIMER_PROF, ITIMER_REAL, ITIMER_VIRTUAL};

#[cfg(target_os = "android")]
pub const ITIMER_REAL: libc::c_int = 0;
#[cfg(target_os = "android")]
pub const ITIMER_VIRTUAL: libc::c_int = 1;
#[cfg(target_os = "android")]
pub const ITIMER_PROF: libc::c_int = 2;

#[cfg(unix)]
#[must_use]
pub fn timeval_to_double(tv: &libc::timeval) -> f64 {
    tv.tv_sec as f64 + (tv.tv_usec as f64 / 1_000_000.0)
}

#[cfg(unix)]
#[must_use]
pub fn double_to_timeval(val: f64) -> libc::timeval {
    libc::timeval {
        tv_sec: val.trunc() as _,
        tv_usec: (val.fract() * 1_000_000.0) as _,
    }
}

#[cfg(unix)]
#[must_use]
pub fn itimerval_to_tuple(it: &libc::itimerval) -> (f64, f64) {
    (
        timeval_to_double(&it.it_value),
        timeval_to_double(&it.it_interval),
    )
}

#[cfg(all(unix, not(target_os = "redox")))]
unsafe extern "C" {
    #[link_name = "siginterrupt"]
    fn c_siginterrupt(sig: i32, flag: i32) -> i32;
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod ffi {
    unsafe extern "C" {
        pub(super) fn getitimer(
            which: libc::c_int,
            curr_value: *mut libc::itimerval,
        ) -> libc::c_int;
        pub(super) fn setitimer(
            which: libc::c_int,
            new_value: *const libc::itimerval,
            old_value: *mut libc::itimerval,
        ) -> libc::c_int;
    }
}

/// # Safety
///
/// The caller must ensure `signalnum` is a valid platform signal number.
#[cfg(any(unix, windows))]
pub unsafe fn probe_handler(signalnum: i32) -> Option<sighandler_t> {
    #[cfg(unix)]
    {
        // Query without changing disposition: even temporarily ignoring SIGCHLD
        // can discard a child's exit status, and signal() also replaces masks.
        let mut action = core::mem::MaybeUninit::<libc::sigaction>::uninit();
        if unsafe { libc::sigaction(signalnum, core::ptr::null(), action.as_mut_ptr()) } == 0 {
            Some(unsafe { action.assume_init() }.sa_sigaction)
        } else {
            None
        }
    }
    #[cfg(windows)]
    {
        let handler = unsafe { libc::signal(signalnum, libc::SIG_IGN) };
        if handler == libc::SIG_ERR as sighandler_t {
            None
        } else {
            unsafe { libc::signal(signalnum, handler) };
            Some(handler)
        }
    }
}

/// # Safety
///
/// The caller must ensure `signalnum` is a valid platform signal number and
/// `handler` is accepted by the platform signal ABI.
#[cfg(any(unix, windows))]
pub unsafe fn install_handler(signalnum: i32, handler: sighandler_t) -> io::Result<sighandler_t> {
    let old = unsafe { libc::signal(signalnum, handler) };
    if old == libc::SIG_ERR as sighandler_t {
        return Err(io::Error::last_os_error());
    }
    #[cfg(all(unix, not(target_os = "redox")))]
    let _ = siginterrupt(signalnum, 1);
    Ok(old)
}

#[cfg(any(unix, windows))]
pub fn raise_signal(signalnum: i32) -> io::Result<()> {
    unsafe { libc::raise(signalnum) }.check_libc_zero()
}

#[cfg(unix)]
pub fn alarm(seconds: u32) -> u32 {
    unsafe { libc::alarm(seconds) }
}

#[cfg(unix)]
pub fn pause() {
    unsafe { libc::pause() };
}

#[cfg(unix)]
pub fn set_sigint_default_onstack() -> io::Result<()> {
    let mut action: libc::sigaction = unsafe { core::mem::zeroed() };
    action.sa_sigaction = libc::SIG_DFL;
    action.sa_flags = libc::SA_ONSTACK;
    unsafe { libc::sigemptyset(&mut action.sa_mask) }.check_libc_zero()?;
    unsafe { libc::sigaction(libc::SIGINT, &action, core::ptr::null_mut()) }.check_libc_zero()
}

#[cfg(unix)]
pub fn send_sigint_to_self() -> io::Result<()> {
    unsafe { libc::kill(libc::getpid(), libc::SIGINT) }.check_libc_zero()
}

#[cfg(unix)]
pub fn setitimer(which: i32, new: &libc::itimerval) -> io::Result<libc::itimerval> {
    let mut old = core::mem::MaybeUninit::<libc::itimerval>::uninit();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let ret = unsafe { ffi::setitimer(which, new, old.as_mut_ptr()) };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let ret = unsafe { libc::setitimer(which, new, old.as_mut_ptr()) };
    ret.check_libc_zero()?;
    Ok(unsafe { old.assume_init() })
}

#[cfg(unix)]
pub fn getitimer(which: i32) -> io::Result<libc::itimerval> {
    let mut old = core::mem::MaybeUninit::<libc::itimerval>::uninit();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let ret = unsafe { ffi::getitimer(which, old.as_mut_ptr()) };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let ret = unsafe { libc::getitimer(which, old.as_mut_ptr()) };
    ret.check_libc_zero()?;
    Ok(unsafe { old.assume_init() })
}

#[cfg(unix)]
pub fn sigemptyset() -> io::Result<libc::sigset_t> {
    let mut set: libc::sigset_t = unsafe { core::mem::zeroed() };
    unsafe { libc::sigemptyset(&mut set) }.check_libc_zero()?;
    Ok(set)
}

#[cfg(unix)]
pub fn sigaddset(set: &mut libc::sigset_t, signum: i32) -> io::Result<()> {
    unsafe { libc::sigaddset(set, signum) }.check_libc_zero()
}

#[cfg(unix)]
pub fn pthread_sigmask(how: i32, set: &libc::sigset_t) -> io::Result<libc::sigset_t> {
    let mut old_mask: libc::sigset_t = unsafe { core::mem::zeroed() };
    let err = unsafe { libc::pthread_sigmask(how, set, &mut old_mask) };
    if err != 0 {
        Err(io::Error::from_raw_os_error(err))
    } else {
        Ok(old_mask)
    }
}

#[cfg(any(target_os = "android", target_os = "linux"))]
pub fn pidfd_send_signal(pidfd: i32, sig: i32, flags: u32) -> io::Result<()> {
    let ret = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd,
            sig,
            core::ptr::null::<libc::siginfo_t>(),
            flags,
        ) as libc::c_long
    };
    ret.check_libc_neg()?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn siginterrupt(signalnum: i32, flag: i32) -> io::Result<()> {
    unsafe { c_siginterrupt(signalnum, flag) }.check_libc_neg()?;
    Ok(())
}

#[cfg(windows)]
pub const VALID_SIGNALS: &[i32] = &[
    libc::SIGINT,
    libc::SIGILL,
    libc::SIGFPE,
    libc::SIGSEGV,
    libc::SIGTERM,
    21, // SIGBREAK / _SIGBREAK
    libc::SIGABRT,
];

#[cfg(windows)]
pub const SIGBREAK: i32 = 21;
#[cfg(windows)]
pub const CTRL_C_EVENT: u32 = 0;
#[cfg(windows)]
pub const CTRL_BREAK_EVENT: u32 = 1;
#[cfg(windows)]
pub const INVALID_SOCKET: libc::SOCKET = windows_sys::Win32::Networking::WinSock::INVALID_SOCKET;

#[cfg(windows)]
fn init_winsock() {
    static WSA_INIT: Once = Once::new();
    WSA_INIT.call_once(|| unsafe {
        let mut wsa_data = core::mem::MaybeUninit::uninit();
        let _ = windows_sys::Win32::Networking::WinSock::WSAStartup(0x0101, wsa_data.as_mut_ptr());
    });
}

#[cfg(windows)]
pub fn wakeup_fd_is_socket(fd: libc::SOCKET) -> io::Result<bool> {
    use windows_sys::Win32::Networking::WinSock;

    init_winsock();
    let mut res = 0i32;
    let mut res_size = core::mem::size_of::<i32>() as i32;
    let getsockopt_res = unsafe {
        WinSock::getsockopt(
            fd,
            WinSock::SOL_SOCKET,
            WinSock::SO_ERROR,
            &mut res as *mut i32 as *mut _,
            &mut res_size,
        )
    };
    if getsockopt_res == 0 {
        return Ok(true);
    }

    let err = io::Error::last_os_error();
    if err.raw_os_error() != Some(WinSock::WSAENOTSOCK) {
        return Err(err);
    }

    let fd_i32 =
        i32::try_from(fd).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid fd"))?;
    let borrowed = unsafe { crate::crt_fd::Borrowed::try_borrow_raw(fd_i32) }?;
    crate::fileutils::fstat(borrowed)?;
    Ok(false)
}

#[cfg(any(unix, windows))]
pub enum WakeupError {
    Write(i32),
    #[cfg(windows)]
    Send(i32),
}

#[cfg(any(unix, windows))]
impl WakeupError {
    pub fn is_would_block(&self) -> bool {
        match *self {
            Self::Write(errno) => errno == libc::EAGAIN || errno == libc::EWOULDBLOCK,
            #[cfg(windows)]
            Self::Send(errno) => errno == windows_sys::Win32::Networking::WinSock::WSAEWOULDBLOCK,
        }
    }

    // Only convert to an allocating error outside the OS signal handler.
    pub fn into_io_error(self) -> io::Error {
        match self {
            #[cfg(unix)]
            Self::Write(errno) => io::Error::from_raw_os_error(errno),
            #[cfg(windows)]
            Self::Write(errno) => crate::os::io_error_from_errno(errno),
            #[cfg(windows)]
            Self::Send(errno) => io::Error::from_raw_os_error(errno),
        }
    }
}

#[cfg(any(unix, windows))]
fn write_signal(signum: i32, wakeup_fd: i32) -> Option<WakeupError> {
    let sigbyte = signum as u8;
    loop {
        if unsafe { libc::write(wakeup_fd, &sigbyte as *const u8 as *const _, 1) } >= 0 {
            return None;
        }
        let errno = crate::os::get_errno();
        if errno != libc::EINTR {
            return Some(WakeupError::Write(errno));
        }
    }
}

#[cfg(windows)]
pub fn notify_signal(
    signum: i32,
    wakeup_fd: libc::SOCKET,
    wakeup_is_socket: bool,
    sigint_event: Option<isize>,
) -> Option<WakeupError> {
    let saved_errno = crate::os::get_errno();
    if signum == libc::SIGINT
        && let Some(handle) = sigint_event
    {
        unsafe {
            windows_sys::Win32::System::Threading::SetEvent(handle as _);
        }
    }

    let error = if wakeup_fd == INVALID_SOCKET {
        None
    } else if wakeup_is_socket {
        let sigbyte = signum as u8;
        let result = unsafe {
            windows_sys::Win32::Networking::WinSock::send(
                wakeup_fd,
                &sigbyte as *const u8 as *const _,
                1,
                0,
            )
        };
        if result < 0 {
            Some(WakeupError::Send(unsafe {
                windows_sys::Win32::Networking::WinSock::WSAGetLastError()
            }))
        } else {
            None
        }
    } else {
        write_signal(signum, wakeup_fd as _)
    };
    crate::os::set_errno(saved_errno);
    error
}

#[cfg(unix)]
pub fn notify_signal(signum: i32, wakeup_fd: i32) -> Option<WakeupError> {
    if wakeup_fd == -1 {
        return None;
    }
    let saved_errno = crate::os::get_errno();
    let error = write_signal(signum, wakeup_fd);
    crate::os::set_errno(saved_errno);
    error
}

#[cfg(target_os = "wasi")]
pub fn notify_signal(signum: i32, wakeup_fd: i32) {
    if wakeup_fd == -1 {
        return;
    }
    let sigbyte = signum as u8;
    unsafe {
        let _ = libc::write(wakeup_fd, &sigbyte as *const u8 as *const _, 1);
    }
}

#[cfg(unix)]
pub fn strsignal(signalnum: i32) -> Option<String> {
    let s = unsafe { libc::strsignal(signalnum) };
    if s.is_null() {
        None
    } else {
        let cstr = unsafe { core::ffi::CStr::from_ptr(s) };
        Some(cstr.to_string_lossy().into_owned())
    }
}

#[cfg(windows)]
pub fn strsignal(signalnum: i32) -> Option<String> {
    let name = match signalnum {
        libc::SIGINT => "Interrupt",
        libc::SIGILL => "Illegal instruction",
        libc::SIGFPE => "Floating-point exception",
        libc::SIGSEGV => "Segmentation fault",
        libc::SIGTERM => "Terminated",
        21 => "Break",
        libc::SIGABRT => "Aborted",
        _ => return None,
    };
    Some(name.to_owned())
}

#[cfg(unix)]
pub fn valid_signals(max_signum: usize) -> io::Result<Vec<i32>> {
    let mut mask: libc::sigset_t = unsafe { core::mem::zeroed() };
    unsafe { libc::sigfillset(&mut mask) }.check_libc_zero()?;
    let mut signals = Vec::new();
    for signum in 1..max_signum {
        if unsafe { libc::sigismember(&mask, signum as i32) } == 1 {
            signals.push(signum as i32);
        }
    }
    Ok(signals)
}

#[cfg(unix)]
pub fn sigset_contains(mask: libc::sigset_t, signum: i32) -> bool {
    unsafe { libc::sigismember(&mask, signum) == 1 }
}

#[cfg(windows)]
pub fn valid_signals(_max_signum: usize) -> io::Result<Vec<i32>> {
    Ok(VALID_SIGNALS.to_vec())
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    #[test]
    fn probing_preserves_signal_flags_and_mask() {
        unsafe extern "C" fn handler(_: libc::c_int) {}

        struct Restore(libc::sigaction);
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe { libc::sigaction(libc::SIGUSR2, &self.0, core::ptr::null_mut()) };
            }
        }

        // No other host_env test uses SIGUSR2; retain the host disposition even
        // when an assertion fails. Do not deliver any signal to the process.
        unsafe {
            let mut action: libc::sigaction = core::mem::zeroed();
            action.sa_sigaction = handler as *const () as libc::sighandler_t;
            action.sa_flags = libc::SA_NODEFER;
            assert_eq!(libc::sigemptyset(&mut action.sa_mask), 0);
            assert_eq!(libc::sigaddset(&mut action.sa_mask, libc::SIGUSR1), 0);
            let mut original = core::mem::MaybeUninit::uninit();
            assert_eq!(
                libc::sigaction(libc::SIGUSR2, &action, original.as_mut_ptr()),
                0
            );
            let _restore = Restore(original.assume_init());

            assert_eq!(
                super::probe_handler(libc::SIGUSR2),
                Some(action.sa_sigaction)
            );
            let mut observed = core::mem::MaybeUninit::<libc::sigaction>::uninit();
            assert_eq!(
                libc::sigaction(libc::SIGUSR2, core::ptr::null(), observed.as_mut_ptr()),
                0
            );
            let observed = observed.assume_init();
            assert_eq!(observed.sa_sigaction, action.sa_sigaction);
            assert_ne!(observed.sa_flags & libc::SA_NODEFER, 0);
            assert_eq!(libc::sigismember(&observed.sa_mask, libc::SIGUSR1), 1);
        }
    }
}
