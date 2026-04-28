use std::io;
#[cfg(windows)]
use std::sync::Once;

#[cfg(any(unix, windows))]
pub use libc::sighandler_t;

#[cfg(unix)]
pub fn timeval_to_double(tv: &libc::timeval) -> f64 {
    tv.tv_sec as f64 + (tv.tv_usec as f64 / 1_000_000.0)
}

#[cfg(unix)]
pub fn double_to_timeval(val: f64) -> libc::timeval {
    libc::timeval {
        tv_sec: val.trunc() as _,
        tv_usec: (val.fract() * 1_000_000.0) as _,
    }
}

#[cfg(unix)]
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
        pub fn getitimer(which: libc::c_int, curr_value: *mut libc::itimerval) -> libc::c_int;
        pub fn setitimer(
            which: libc::c_int,
            new_value: *const libc::itimerval,
            old_value: *mut libc::itimerval,
        ) -> libc::c_int;
    }
}

#[cfg(any(unix, windows))]
/// # Safety
///
/// The caller must ensure `signalnum` is a valid platform signal number.
pub unsafe fn probe_handler(signalnum: i32) -> Option<sighandler_t> {
    let handler = unsafe { libc::signal(signalnum, libc::SIG_IGN) };
    if handler == libc::SIG_ERR as sighandler_t {
        None
    } else {
        unsafe { libc::signal(signalnum, handler) };
        Some(handler)
    }
}

#[cfg(any(unix, windows))]
/// # Safety
///
/// The caller must ensure `signalnum` is a valid platform signal number and
/// `handler` is accepted by the platform signal ABI.
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
    let res = unsafe { libc::raise(signalnum) };
    if res != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
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
    if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::sigaction(libc::SIGINT, &action, core::ptr::null_mut()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
pub fn send_sigint_to_self() -> io::Result<()> {
    if unsafe { libc::kill(libc::getpid(), libc::SIGINT) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
pub fn setitimer(which: i32, new: &libc::itimerval) -> io::Result<libc::itimerval> {
    let mut old = core::mem::MaybeUninit::<libc::itimerval>::uninit();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let ret = unsafe { ffi::setitimer(which, new, old.as_mut_ptr()) };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let ret = unsafe { libc::setitimer(which, new, old.as_mut_ptr()) };
    if ret != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { old.assume_init() })
    }
}

#[cfg(unix)]
pub fn getitimer(which: i32) -> io::Result<libc::itimerval> {
    let mut old = core::mem::MaybeUninit::<libc::itimerval>::uninit();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let ret = unsafe { ffi::getitimer(which, old.as_mut_ptr()) };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let ret = unsafe { libc::getitimer(which, old.as_mut_ptr()) };
    if ret != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { old.assume_init() })
    }
}

#[cfg(unix)]
pub fn sigemptyset() -> io::Result<libc::sigset_t> {
    let mut set: libc::sigset_t = unsafe { core::mem::zeroed() };
    if unsafe { libc::sigemptyset(&mut set) } != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(set)
    }
}

#[cfg(unix)]
pub fn sigaddset(set: &mut libc::sigset_t, signum: i32) -> io::Result<()> {
    if unsafe { libc::sigaddset(set, signum) } != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
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

#[cfg(target_os = "linux")]
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
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn siginterrupt(signalnum: i32, flag: i32) -> io::Result<()> {
    let res = unsafe { c_siginterrupt(signalnum, flag) };
    if res < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
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
pub fn is_valid_signal(signalnum: i32) -> bool {
    VALID_SIGNALS.contains(&signalnum)
}

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

#[cfg(windows)]
pub fn notify_signal(
    signum: i32,
    wakeup_fd: libc::SOCKET,
    wakeup_is_socket: bool,
    sigint_event: Option<isize>,
) {
    if signum == libc::SIGINT
        && let Some(handle) = sigint_event
    {
        unsafe {
            windows_sys::Win32::System::Threading::SetEvent(handle as _);
        }
    }

    if wakeup_fd == INVALID_SOCKET {
        return;
    }

    let sigbyte = signum as u8;
    if wakeup_is_socket {
        unsafe {
            let _ = windows_sys::Win32::Networking::WinSock::send(
                wakeup_fd,
                &sigbyte as *const u8 as *const _,
                1,
                0,
            );
        }
    } else {
        unsafe {
            let _ = libc::write(wakeup_fd as _, &sigbyte as *const u8 as *const _, 1);
        }
    }
}

#[cfg(unix)]
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
    if unsafe { libc::sigfillset(&mut mask) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut signals = Vec::new();
    for signum in 1..max_signum {
        if unsafe { libc::sigismember(&mask, signum as i32) } == 1 {
            signals.push(signum as i32);
        }
    }
    Ok(signals)
}

#[cfg(unix)]
pub fn sigset_contains(mask: &libc::sigset_t, signum: i32) -> bool {
    unsafe { libc::sigismember(mask, signum) == 1 }
}

#[cfg(windows)]
pub fn valid_signals(_max_signum: usize) -> io::Result<Vec<i32>> {
    Ok(VALID_SIGNALS.to_vec())
}
