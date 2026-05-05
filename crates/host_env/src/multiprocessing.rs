#![allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "Semaphore helpers intentionally mirror OS handle and pointer APIs."
)]
#![allow(
    clippy::result_unit_err,
    reason = "These helpers preserve the existing host-facing error surface."
)]

#[cfg(unix)]
use alloc::ffi::CString;
#[cfg(windows)]
use std::io;

#[cfg(unix)]
use libc::sem_t;
#[cfg(unix)]
use nix::errno::Errno;

#[cfg(unix)]
#[derive(Debug)]
pub struct SemHandle {
    raw: *mut sem_t,
}

#[cfg(unix)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SemError {
    WouldBlock,
    TimedOut,
    Interrupted,
    AlreadyExists,
    NotFound,
    InvalidInput,
    Other(i32),
}

#[cfg(unix)]
impl SemError {
    fn from_errno(err: Errno) -> Self {
        match err {
            Errno::EAGAIN => Self::WouldBlock,
            Errno::ETIMEDOUT => Self::TimedOut,
            Errno::EINTR => Self::Interrupted,
            Errno::EEXIST => Self::AlreadyExists,
            Errno::ENOENT => Self::NotFound,
            Errno::EINVAL => Self::InvalidInput,
            other => Self::Other(other as i32),
        }
    }

    pub fn raw_os_error(self) -> i32 {
        match self {
            Self::WouldBlock => Errno::EAGAIN as i32,
            Self::TimedOut => Errno::ETIMEDOUT as i32,
            Self::Interrupted => Errno::EINTR as i32,
            Self::AlreadyExists => Errno::EEXIST as i32,
            Self::NotFound => Errno::ENOENT as i32,
            Self::InvalidInput => Errno::EINVAL as i32,
            Self::Other(code) => code,
        }
    }

    pub fn description(self) -> String {
        Errno::from_raw(self.raw_os_error()).desc().to_owned()
    }
}

#[cfg(unix)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TryAcquireStatus {
    Acquired,
    WouldBlock,
    Interrupted,
    Error(SemError),
}

#[cfg(unix)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WaitStatus {
    Acquired,
    TimedOut,
    Interrupted,
    Error(SemError),
}

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_TOO_MANY_POSTS, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Networking::WinSock::{SOCKET, closesocket, recv, send},
    System::Threading::{
        CreateSemaphoreW, GetCurrentThreadId, INFINITE, ReleaseSemaphore, WaitForSingleObjectEx,
    },
};

#[cfg(windows)]
pub type RawHandle = HANDLE;
#[cfg(windows)]
pub type RawSocket = SOCKET;
#[cfg(windows)]
pub const INFINITE_TIMEOUT: u32 = INFINITE;

#[cfg(windows)]
#[derive(Debug)]
pub struct SemHandle {
    raw: HANDLE,
}

unsafe impl Send for SemHandle {}
unsafe impl Sync for SemHandle {}

#[cfg(unix)]
impl SemHandle {
    pub fn create(
        name: &str,
        value: u32,
        unlink: bool,
    ) -> Result<(Self, Option<String>), SemError> {
        let cname = semaphore_name(name).map_err(|_| SemError::InvalidInput)?;
        let raw =
            unsafe { libc::sem_open(cname.as_ptr(), libc::O_CREAT | libc::O_EXCL, 0o600, value) };
        if raw == libc::SEM_FAILED {
            return Err(SemError::from_errno(Errno::last()));
        }
        if unlink {
            unsafe {
                libc::sem_unlink(cname.as_ptr());
            }
            Ok((Self { raw }, None))
        } else {
            Ok((Self { raw }, Some(name.to_owned())))
        }
    }

    pub fn open_existing(name: &str) -> Result<Self, SemError> {
        let cname = semaphore_name(name).map_err(|_| SemError::InvalidInput)?;
        let raw = unsafe { libc::sem_open(cname.as_ptr(), 0) };
        if raw == libc::SEM_FAILED {
            Err(SemError::from_errno(Errno::last()))
        } else {
            Ok(Self { raw })
        }
    }

    #[inline]
    pub fn as_ptr(&self) -> *mut sem_t {
        self.raw
    }
}

#[cfg(windows)]
impl SemHandle {
    pub fn create(value: i32, maxvalue: i32) -> io::Result<Self> {
        let handle =
            unsafe { CreateSemaphoreW(core::ptr::null(), value, maxvalue, core::ptr::null()) };
        if handle == 0 as HANDLE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self { raw: handle })
        }
    }

    #[inline]
    pub fn from_raw(raw: HANDLE) -> Self {
        Self { raw }
    }

    #[inline]
    pub fn as_raw(&self) -> HANDLE {
        self.raw
    }
}

#[cfg(unix)]
impl Drop for SemHandle {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                libc::sem_close(self.raw);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for SemHandle {
    fn drop(&mut self) {
        if self.raw != 0 as HANDLE && self.raw != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.raw);
            }
        }
    }
}

#[cfg(unix)]
#[inline]
pub fn current_thread_id() -> u64 {
    unsafe { libc::pthread_self() as u64 }
}

#[cfg(windows)]
#[inline]
pub fn current_thread_id() -> u32 {
    unsafe { GetCurrentThreadId() }
}

#[cfg(windows)]
#[inline]
pub fn wait_for_single_object(handle: HANDLE, timeout_ms: u32) -> u32 {
    unsafe { WaitForSingleObjectEx(handle, timeout_ms, 0) }
}

#[cfg(windows)]
#[inline]
pub fn wait_object_0() -> u32 {
    WAIT_OBJECT_0
}

#[cfg(windows)]
#[inline]
pub fn wait_timeout() -> u32 {
    WAIT_TIMEOUT
}

#[cfg(windows)]
#[inline]
pub fn close_socket(socket: SOCKET) -> io::Result<()> {
    let res = unsafe { closesocket(socket) };
    if res != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
pub fn recv_socket(socket: SOCKET, size: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; size];
    let n_read = unsafe { recv(socket, buf.as_mut_ptr() as *mut _, size as i32, 0) };
    if n_read < 0 {
        Err(io::Error::last_os_error())
    } else {
        buf.truncate(n_read as usize);
        Ok(buf)
    }
}

#[cfg(windows)]
pub fn send_socket(socket: SOCKET, buf: &[u8]) -> io::Result<i32> {
    let ret = unsafe { send(socket, buf.as_ptr() as *const _, buf.len() as i32, 0) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

#[cfg(windows)]
#[inline]
pub fn wait_failed() -> u32 {
    WAIT_FAILED
}

#[cfg(windows)]
pub fn release_semaphore(handle: HANDLE) -> Result<(), u32> {
    if unsafe { ReleaseSemaphore(handle, 1, core::ptr::null_mut()) } == 0 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

#[cfg(windows)]
pub fn get_semaphore_value(handle: HANDLE) -> Result<i32, ()> {
    match wait_for_single_object(handle, 0) {
        WAIT_OBJECT_0 => {
            let mut previous: i32 = 0;
            if unsafe { ReleaseSemaphore(handle, 1, &mut previous) } == 0 {
                Err(())
            } else {
                Ok(previous + 1)
            }
        }
        WAIT_TIMEOUT => Ok(0),
        _ => Err(()),
    }
}

#[cfg(windows)]
#[inline]
pub fn is_too_many_posts(err: u32) -> bool {
    err == ERROR_TOO_MANY_POSTS
}

#[cfg(unix)]
pub fn semaphore_name(name: &str) -> Result<CString, alloc::ffi::NulError> {
    let mut full = String::with_capacity(name.len() + 1);
    if !name.starts_with('/') {
        full.push('/');
    }
    full.push_str(name);
    CString::new(full)
}

#[cfg(unix)]
pub fn sem_unlink(name: &str) -> Result<(), SemError> {
    let cname = semaphore_name(name).map_err(|_| SemError::InvalidInput)?;
    let res = unsafe { libc::sem_unlink(cname.as_ptr()) };
    if res < 0 {
        Err(SemError::from_errno(Errno::last()))
    } else {
        Ok(())
    }
}

#[cfg(all(unix, not(target_vendor = "apple")))]
/// # Safety
///
/// `handle` must point to a valid `sem_t` that remains alive for the duration
/// of this call and is valid to pass to `sem_getvalue`.
pub unsafe fn get_semaphore_value(handle: *mut sem_t) -> Result<i32, SemError> {
    let mut sval: libc::c_int = 0;
    let res = unsafe { libc::sem_getvalue(handle, &mut sval) };
    if res < 0 {
        Err(SemError::from_errno(Errno::last()))
    } else {
        Ok(if sval < 0 { 0 } else { sval })
    }
}

#[cfg(unix)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sem_trywait_status(handle: *mut sem_t) -> TryAcquireStatus {
    if unsafe { libc::sem_trywait(handle) } == 0 {
        TryAcquireStatus::Acquired
    } else {
        match Errno::last() {
            Errno::EAGAIN => TryAcquireStatus::WouldBlock,
            Errno::EINTR => TryAcquireStatus::Interrupted,
            err => TryAcquireStatus::Error(SemError::from_errno(err)),
        }
    }
}

#[cfg(unix)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sem_post(handle: *mut sem_t) -> Result<(), SemError> {
    if unsafe { libc::sem_post(handle) } < 0 {
        Err(SemError::from_errno(Errno::last()))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub fn sem_value_max() -> i32 {
    let val = unsafe { libc::sysconf(libc::_SC_SEM_VALUE_MAX) };
    if val < 0 || val > i32::MAX as libc::c_long {
        i32::MAX
    } else {
        val as i32
    }
}

#[cfg(unix)]
pub fn gettimeofday() -> Result<libc::timeval, SemError> {
    let mut tv = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    if unsafe { libc::gettimeofday(&mut tv, core::ptr::null_mut()) } < 0 {
        Err(SemError::from_errno(Errno::last()))
    } else {
        Ok(tv)
    }
}

#[cfg(unix)]
pub fn deadline_from_timeout(timeout: f64) -> Result<libc::timespec, SemError> {
    let timeout = if timeout < 0.0 { 0.0 } else { timeout };
    let tv = gettimeofday()?;
    let sec = timeout as libc::c_long;
    let nsec = (1e9 * (timeout - sec as f64) + 0.5) as libc::c_long;
    let mut deadline = libc::timespec {
        tv_sec: tv.tv_sec + sec as libc::time_t,
        tv_nsec: (tv.tv_usec as libc::c_long * 1000 + nsec) as _,
    };
    deadline.tv_sec += (deadline.tv_nsec / 1_000_000_000) as libc::time_t;
    deadline.tv_nsec %= 1_000_000_000;
    Ok(deadline)
}

#[cfg(unix)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sem_wait_status(handle: *mut sem_t, deadline: Option<&libc::timespec>) -> WaitStatus {
    #[cfg(not(target_vendor = "apple"))]
    if let Some(deadline) = deadline {
        if unsafe { libc::sem_timedwait(handle, deadline) } == 0 {
            WaitStatus::Acquired
        } else {
            match Errno::last() {
                Errno::ETIMEDOUT => WaitStatus::TimedOut,
                Errno::EINTR => WaitStatus::Interrupted,
                err => WaitStatus::Error(SemError::from_errno(err)),
            }
        }
    } else {
        if unsafe { libc::sem_wait(handle) } == 0 {
            WaitStatus::Acquired
        } else {
            match Errno::last() {
                Errno::EINTR => WaitStatus::Interrupted,
                err => WaitStatus::Error(SemError::from_errno(err)),
            }
        }
    }

    #[cfg(target_vendor = "apple")]
    {
        debug_assert!(deadline.is_none());
        if unsafe { libc::sem_wait(handle) } == 0 {
            WaitStatus::Acquired
        } else {
            match Errno::last() {
                Errno::EINTR => WaitStatus::Interrupted,
                err => WaitStatus::Error(SemError::from_errno(err)),
            }
        }
    }
}

#[cfg(target_vendor = "apple")]
pub enum PollWaitStep {
    Acquired,
    Timeout,
    Continue(u64),
}

#[cfg(target_vendor = "apple")]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn sem_timedwait_poll_step(
    handle: *mut sem_t,
    deadline: &libc::timespec,
    delay: u64,
) -> Result<PollWaitStep, SemError> {
    if unsafe { libc::sem_trywait(handle) } == 0 {
        return Ok(PollWaitStep::Acquired);
    }
    let err = Errno::last();
    if err != Errno::EAGAIN {
        return Err(SemError::from_errno(err));
    }

    let now = gettimeofday()?;
    let deadline_usec = deadline.tv_sec * 1_000_000 + deadline.tv_nsec / 1000;
    #[allow(clippy::unnecessary_cast)]
    let now_usec = now.tv_sec as i64 * 1_000_000 + now.tv_usec as i64;
    if now_usec >= deadline_usec {
        return Ok(PollWaitStep::Timeout);
    }

    let difference = (deadline_usec - now_usec) as u64;
    let mut delay = delay + 1000;
    if delay > 20000 {
        delay = 20000;
    }
    if delay > difference {
        delay = difference;
    }

    let mut tv_delay = libc::timeval {
        tv_sec: (delay / 1_000_000) as _,
        tv_usec: (delay % 1_000_000) as _,
    };
    unsafe {
        libc::select(
            0,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut tv_delay,
        );
    }
    Ok(PollWaitStep::Continue(delay))
}
