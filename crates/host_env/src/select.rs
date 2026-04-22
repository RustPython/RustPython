use core::mem::MaybeUninit;
use std::io;

#[cfg(unix)]
mod platform {
    pub use libc::pollfd;
    pub use libc::{FD_ISSET, FD_SET, FD_SETSIZE, FD_ZERO, fd_set, select, timeval};
    pub use std::os::unix::io::RawFd;

    #[must_use]
    pub const fn check_err(x: i32) -> bool {
        x < 0
    }
}

#[allow(non_snake_case)]
#[cfg(windows)]
mod platform {
    pub use WinSock::{FD_SET as fd_set, FD_SETSIZE, SOCKET as RawFd, TIMEVAL as timeval, select};
    use windows_sys::Win32::Networking::WinSock;

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

    pub unsafe fn FD_ZERO(set: *mut fd_set) {
        unsafe { (*set).fd_count = 0 };
    }

    pub unsafe fn FD_ISSET(fd: RawFd, set: *mut fd_set) -> bool {
        use WinSock::__WSAFDIsSet;
        unsafe { __WSAFDIsSet(fd as _, set) != 0 }
    }

    #[must_use]
    pub fn check_err(x: i32) -> bool {
        x == WinSock::SOCKET_ERROR
    }
}

#[cfg(target_os = "wasi")]
mod platform {
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
        set.__nfds = n + 1;
        set.__fds[n] = fd;
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

#[cfg(unix)]
pub type PollFd = platform::pollfd;

#[repr(transparent)]
pub struct FdSet(MaybeUninit<platform::fd_set>);

impl FdSet {
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

pub fn sec_to_timeval(sec: f64) -> timeval {
    timeval {
        tv_sec: sec.trunc() as _,
        tv_usec: (sec.fract() * 1e6) as _,
    }
}

#[cfg(unix)]
#[inline]
pub fn search_poll_fd(fds: &[PollFd], fd: i32) -> Result<usize, usize> {
    fds.binary_search_by_key(&fd, |pfd| pfd.fd)
}

#[cfg(unix)]
pub fn insert_poll_fd(fds: &mut Vec<PollFd>, fd: i32, events: i16) {
    match search_poll_fd(fds, fd) {
        Ok(i) => fds[i].events = events,
        Err(i) => fds.insert(
            i,
            PollFd {
                fd,
                events,
                revents: 0,
            },
        ),
    }
}

#[cfg(unix)]
pub fn get_poll_fd_mut(fds: &mut [PollFd], fd: i32) -> Option<&mut PollFd> {
    search_poll_fd(fds, fd).ok().map(move |i| &mut fds[i])
}

#[cfg(unix)]
pub fn remove_poll_fd(fds: &mut Vec<PollFd>, fd: i32) -> Option<PollFd> {
    search_poll_fd(fds, fd).ok().map(|i| fds.remove(i))
}

#[cfg(unix)]
pub fn poll_fds(fds: &mut [PollFd], timeout: i32) -> std::io::Result<i32> {
    let res = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
    if res < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(res)
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "redox"))]
pub mod epoll {
    use std::os::fd::{AsFd, IntoRawFd, OwnedFd};

    pub use rustix::event::Timespec;
    pub use rustix::event::epoll::{Event, EventData, EventFlags};

    #[derive(Debug)]
    pub enum WaitError {
        Interrupted,
        Io(std::io::Error),
    }

    pub fn create() -> std::io::Result<OwnedFd> {
        rustix::event::epoll::create(rustix::event::epoll::CreateFlags::CLOEXEC).map_err(Into::into)
    }

    pub fn close(fd: OwnedFd) -> nix::Result<()> {
        nix::unistd::close(fd.into_raw_fd())
    }

    pub fn add<F: AsFd>(epoll: &OwnedFd, fd: F, data: u64, events: u32) -> std::io::Result<()> {
        rustix::event::epoll::add(
            epoll,
            fd,
            EventData::new_u64(data),
            EventFlags::from_bits_retain(events),
        )
        .map_err(Into::into)
    }

    pub fn modify<F: AsFd>(epoll: &OwnedFd, fd: F, data: u64, events: u32) -> std::io::Result<()> {
        rustix::event::epoll::modify(
            epoll,
            fd,
            EventData::new_u64(data),
            EventFlags::from_bits_retain(events),
        )
        .map_err(Into::into)
    }

    pub fn delete<F: AsFd>(epoll: &OwnedFd, fd: F) -> std::io::Result<()> {
        rustix::event::epoll::delete(epoll, fd).map_err(Into::into)
    }

    pub fn wait(
        epoll: &OwnedFd,
        events: &mut Vec<Event>,
        timeout: Option<&Timespec>,
    ) -> Result<usize, WaitError> {
        events.clear();
        match rustix::event::epoll::wait(epoll, rustix::buffer::spare_capacity(events), timeout) {
            Ok(n) => Ok(n),
            Err(rustix::io::Errno::INTR) => Err(WaitError::Interrupted),
            Err(err) => Err(WaitError::Io(err.into())),
        }
    }
}
