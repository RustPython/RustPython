use core::mem::MaybeUninit;
use std::io;

#[cfg(unix)]
pub mod platform {
    pub use libc::{FD_ISSET, FD_SET, FD_SETSIZE, FD_ZERO, fd_set, select, timeval};
    pub use std::os::unix::io::RawFd;

    #[must_use] 
    pub const fn check_err(x: i32) -> bool {
        x < 0
    }
}

#[allow(non_snake_case)]
#[cfg(windows)]
pub mod platform {
    pub use WinSock::{FD_SET as fd_set, FD_SETSIZE, SOCKET as RawFd, TIMEVAL as timeval, select};
    use windows_sys::Win32::Networking::WinSock;

    /// # Safety
    ///
    /// Requirements forwarded from the caller.
    pub unsafe fn FD_SET(fd: RawFd, set: *mut fd_set) {
        let mut slot = unsafe { (&raw mut (*set).fd_array).cast::<RawFd>() };
        let fd_count = unsafe { (*set).fd_count };
        for _ in 0..fd_count {
            if unsafe { *slot } == fd {
                return;
            }
            slot = unsafe { slot.add(1) };
        }
        if fd_count < FD_SETSIZE {
            unsafe {
                *slot = fd as RawFd;
                (*set).fd_count += 1;
            }
        }
    }

    /// # Safety
    ///
    /// Requirements forwarded from the caller.
    pub unsafe fn FD_ZERO(set: *mut fd_set) {
        unsafe { (*set).fd_count = 0 };
    }

    /// # Safety
    ///
    /// Requirements forwarded from the caller.
    pub unsafe fn FD_ISSET(fd: RawFd, set: *mut fd_set) -> bool {
        use WinSock::__WSAFDIsSet;
        unsafe { __WSAFDIsSet(fd as _, set) != 0 }
    }

    pub fn check_err(x: i32) -> bool {
        x == WinSock::SOCKET_ERROR
    }
}

#[cfg(target_os = "wasi")]
pub mod platform {
    pub use libc::{FD_SETSIZE, timeval};
    pub use std::os::fd::RawFd;

    pub const fn check_err(x: i32) -> bool {
        x < 0
    }

    #[repr(C)]
    pub struct fd_set {
        __nfds: usize,
        __fds: [libc::c_int; FD_SETSIZE],
    }

    #[allow(non_snake_case)]
    pub unsafe fn FD_ISSET(fd: RawFd, set: *const fd_set) -> bool {
        let set = unsafe { &*set };
        for p in &set.__fds[..set.__nfds] {
            if *p == fd {
                return true;
            }
        }
        false
    }

    #[allow(non_snake_case)]
    pub unsafe fn FD_SET(fd: RawFd, set: *mut fd_set) {
        let set = unsafe { &mut *set };
        for p in &set.__fds[..set.__nfds] {
            if *p == fd {
                return;
            }
        }
        let n = set.__nfds;
        assert!(n < set.__fds.len(), "fd_set full");
        set.__fds[n] = fd;
        set.__nfds = n + 1;
    }

    #[allow(non_snake_case)]
    pub unsafe fn FD_ZERO(set: *mut fd_set) {
        unsafe { (*set).__nfds = 0 };
    }

    unsafe extern "C" {
        pub fn select(
            nfds: libc::c_int,
            readfds: *mut fd_set,
            writefds: *mut fd_set,
            errorfds: *mut fd_set,
            timeout: *const timeval,
        ) -> libc::c_int;
    }
}

pub use platform::{RawFd, timeval};

#[repr(transparent)]
pub struct FdSet(MaybeUninit<platform::fd_set>);

impl FdSet {
    #[must_use]
    pub fn new() -> Self {
        let mut fdset = MaybeUninit::zeroed();
        unsafe { platform::FD_ZERO(fdset.as_mut_ptr()) };
        Self(fdset)
    }

    pub fn insert(&mut self, fd: RawFd) {
        unsafe { platform::FD_SET(fd, self.0.as_mut_ptr()) };
    }

    pub fn contains(&mut self, fd: RawFd) -> bool {
        unsafe { platform::FD_ISSET(fd, self.0.as_mut_ptr()) }
    }

    pub fn clear(&mut self) {
        unsafe { platform::FD_ZERO(self.0.as_mut_ptr()) };
    }

    pub fn highest(&mut self) -> Option<RawFd> {
        (0..platform::FD_SETSIZE as RawFd)
            .rev()
            .find(|&fd| self.contains(fd))
    }
}

impl Default for FdSet {
    fn default() -> Self {
        Self::new()
    }
}

pub fn select(
    nfds: libc::c_int,
    readfds: &mut FdSet,
    writefds: &mut FdSet,
    errfds: &mut FdSet,
    timeout: Option<&mut timeval>,
) -> io::Result<i32> {
    let timeout = match timeout {
        Some(tv) => tv as *mut timeval,
        None => core::ptr::null_mut(),
    };
    let ret = unsafe {
        platform::select(
            nfds,
            readfds.0.as_mut_ptr(),
            writefds.0.as_mut_ptr(),
            errfds.0.as_mut_ptr(),
            timeout,
        )
    };
    if platform::check_err(ret) {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

#[must_use]
pub fn sec_to_timeval(sec: f64) -> timeval {
    timeval {
        tv_sec: sec.trunc() as _,
        tv_usec: (sec.fract() * 1e6) as _,
    }
}
