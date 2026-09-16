use core::mem::MaybeUninit;
use core::time::Duration;
use std::io;
use std::time::Instant;

#[cfg(unix)]
pub use libc::{
    EINTR, FD_SETSIZE, PIPE_BUF, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT, POLLPRI, POLLRDBAND,
    POLLRDNORM, POLLWRBAND, POLLWRNORM,
};

#[cfg(any(target_os = "linux", target_os = "android", target_os = "redox"))]
pub use libc::{
    EPOLL_CLOEXEC, EPOLLERR, EPOLLET, EPOLLEXCLUSIVE, EPOLLHUP, EPOLLIN, EPOLLMSG, EPOLLONESHOT,
    EPOLLOUT, EPOLLPRI, EPOLLRDBAND, EPOLLRDHUP, EPOLLRDNORM, EPOLLWAKEUP, EPOLLWRBAND,
    EPOLLWRNORM,
};

#[cfg(unix)]
pub mod platform {
    pub use libc::pollfd;
    pub use libc::{FD_ISSET, FD_SET, FD_SETSIZE, FD_ZERO, fd_set, select, timeval};
    use std::io;
    pub use std::os::unix::io::RawFd;

    #[must_use]
    pub const fn check_err(x: i32) -> bool {
        x < 0
    }

    pub fn last_select_error() -> io::Error {
        io::Error::last_os_error()
    }
}

#[allow(non_snake_case)]
#[cfg(windows)]
pub mod platform {
    pub use WinSock::{FD_SET as fd_set, FD_SETSIZE, SOCKET as RawFd, TIMEVAL as timeval, select};
    use std::io;
    use windows_sys::Win32::Networking::WinSock;

    /// # Safety
    ///
    /// `set` must be a valid mutable pointer to an initialized WinSock fd_set.
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
    /// `set` must be a valid mutable pointer to a WinSock fd_set.
    pub unsafe fn FD_ZERO(set: *mut fd_set) {
        unsafe { (*set).fd_count = 0 };
    }

    /// # Safety
    ///
    /// `set` must be a valid mutable pointer to an initialized WinSock fd_set.
    pub unsafe fn FD_ISSET(fd: RawFd, set: *mut fd_set) -> bool {
        use WinSock::__WSAFDIsSet;
        unsafe { __WSAFDIsSet(fd as _, set) != 0 }
    }

    #[must_use]
    pub fn check_err(x: i32) -> bool {
        x == WinSock::SOCKET_ERROR
    }

    pub fn last_select_error() -> io::Error {
        io::Error::from_raw_os_error(unsafe { WinSock::WSAGetLastError() })
    }
}

#[cfg(target_os = "wasi")]
pub mod platform {
    pub use libc::{FD_SETSIZE, timeval};
    use std::io;
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
    /// # Safety
    ///
    /// `set` must be a valid pointer to an initialized fd_set.
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
    /// # Safety
    ///
    /// `set` must be a valid mutable pointer to an initialized fd_set.
    pub unsafe fn FD_SET(fd: RawFd, set: *mut fd_set) {
        let set = unsafe { &mut *set };
        for p in &set.__fds[..set.__nfds] {
            if *p == fd {
                return;
            }
        }
        let n = set.__nfds;
        if n < FD_SETSIZE {
            set.__fds[n] = fd;
            set.__nfds = n + 1;
        }
    }

    #[allow(non_snake_case)]
    /// # Safety
    ///
    /// `set` must be a valid mutable pointer to an fd_set.
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

    pub fn last_select_error() -> io::Error {
        io::Error::last_os_error()
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
        Err(platform::last_select_error())
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

pub fn duration_to_timeval(d: Duration) -> timeval {
    timeval {
        tv_sec: d.as_secs() as _,
        tv_usec: d.subsec_micros() as _,
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WaitKind {
    Read,
    Write,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WaitFd {
    Ready,
    Timeout,
}

#[derive(Debug)]
pub enum WaitFdError {
    Interrupted,
    Io(io::Error),
}

pub fn wait_fd(
    fd: RawFd,
    kind: WaitKind,
    deadline: Option<Instant>,
) -> Result<WaitFd, WaitFdError> {
    #[cfg(unix)]
    {
        wait_fd_poll(fd, kind, deadline)
    }
    #[cfg(not(unix))]
    {
        wait_fd_select(fd, kind, deadline)
    }
}

#[cfg(unix)]
fn wait_fd_poll(
    fd: RawFd,
    kind: WaitKind,
    deadline: Option<Instant>,
) -> Result<WaitFd, WaitFdError> {
    let events = match kind {
        WaitKind::Read => POLLIN | POLLPRI,
        WaitKind::Write => POLLOUT,
    };
    let mut fds = [PollFd {
        fd,
        events,
        revents: 0,
    }];
    loop {
        let (timeout, is_capped) = match deadline {
            None => (-1, false),
            Some(deadline) => {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    return Ok(WaitFd::Timeout);
                };
                let ms = remaining.as_millis();
                if ms > i32::MAX as u128 {
                    (i32::MAX, true)
                } else {
                    (ms as i32, false)
                }
            }
        };
        match poll_fds(&mut fds, timeout) {
            Ok(0) if is_capped => {}
            Ok(0) => return Ok(WaitFd::Timeout),
            Ok(_) if fds[0].revents & POLLNVAL != 0 => {
                return Err(WaitFdError::Io(io::Error::from_raw_os_error(libc::EBADF)));
            }
            Ok(_) => return Ok(WaitFd::Ready),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                return Err(WaitFdError::Interrupted);
            }
            Err(err) => return Err(WaitFdError::Io(err)),
        }
    }
}

#[cfg(not(unix))]
fn wait_fd_select(
    fd: RawFd,
    kind: WaitKind,
    deadline: Option<Instant>,
) -> Result<WaitFd, WaitFdError> {
    let mut reads = FdSet::new();
    let mut writes = FdSet::new();
    let mut errs = FdSet::new();
    match kind {
        WaitKind::Read => {
            reads.insert(fd);
            errs.insert(fd);
        }
        WaitKind::Write => {
            writes.insert(fd);
            errs.insert(fd);
        }
    }
    let mut timeout = match deadline {
        None => None,
        Some(deadline) => {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(WaitFd::Timeout);
            };
            Some(duration_to_timeval(remaining))
        }
    };
    let nfds = cfg_select! {
        windows => 0,
        _ => fd.saturating_add(1) as libc::c_int,
    };
    match select(nfds, &mut reads, &mut writes, &mut errs, timeout.as_mut()) {
        Ok(0) => Ok(WaitFd::Timeout),
        Ok(_) => Ok(WaitFd::Ready),
        Err(err) if err.kind() == io::ErrorKind::Interrupted => Err(WaitFdError::Interrupted),
        Err(err) => Err(WaitFdError::Io(err)),
    }
}

/// Convert a duration to a `poll(2)` millisecond timeout, rounding toward +∞.
#[cfg(unix)]
pub fn duration_as_millis_ceiling(d: Duration) -> Option<i32> {
    let mut ms = d.as_millis();
    if Duration::from_millis(ms.min(u128::from(u64::MAX)) as u64) < d {
        ms = ms.saturating_add(1);
    }
    i32::try_from(ms).ok()
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
            Ok(n) => {
                unsafe { events.set_len(n) };
                Ok(n)
            }
            Err(rustix::io::Errno::INTR) => Err(WaitError::Interrupted),
            Err(err) => Err(WaitError::Io(err.into())),
        }
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly",
))]
pub mod kqueue {
    use alloc::sync::{Arc, Weak};
    use core::sync::atomic::{AtomicI32, Ordering};
    use core::time::Duration;
    use parking_lot::Mutex;
    use std::io;
    use std::os::fd::BorrowedFd;

    pub use libc::{
        EV_ADD, EV_CLEAR, EV_DELETE, EV_DISABLE, EV_ENABLE, EV_EOF, EV_ERROR, EV_FLAG1, EV_ONESHOT,
        EV_SYSFLAGS, EVFILT_AIO, EVFILT_PROC, EVFILT_READ, EVFILT_SIGNAL, EVFILT_TIMER,
        EVFILT_VNODE, EVFILT_WRITE, NOTE_ATTRIB, NOTE_CHILD, NOTE_DELETE, NOTE_EXEC, NOTE_EXIT,
        NOTE_EXTEND, NOTE_FORK, NOTE_LINK, NOTE_LOWAT, NOTE_PCTRLMASK, NOTE_PDATAMASK, NOTE_RENAME,
        NOTE_REVOKE, NOTE_TRACK, NOTE_TRACKERR, NOTE_WRITE,
    };

    // NetBSD widths differ from i16/u16; the cast is a no-op elsewhere.
    #[allow(clippy::unnecessary_cast)]
    pub const DEFAULT_FILTER: i16 = EVFILT_READ as i16;
    #[allow(clippy::unnecessary_cast)]
    pub const DEFAULT_FLAGS: u16 = EV_ADD as u16;

    #[derive(Copy, Clone, Debug)]
    pub struct Timespec {
        pub sec: i64,
        pub nsec: i64,
    }

    impl Timespec {
        pub fn from_secs(secs: f64) -> Option<Self> {
            if !secs.is_finite() || secs > i64::MAX as f64 {
                return None;
            }
            let mut sec = secs.trunc() as i64;
            let mut nsec = ((secs - sec as f64) * 1e9).round() as i64;
            if nsec >= 1_000_000_000 {
                sec = sec.saturating_add(1);
                nsec -= 1_000_000_000;
            }
            Some(Self { sec, nsec })
        }

        pub fn from_duration(d: Duration) -> Self {
            Self {
                sec: d.as_secs() as i64,
                nsec: i64::from(d.subsec_nanos()),
            }
        }

        pub fn to_duration(self) -> Option<Duration> {
            if self.sec < 0 || !(0..1_000_000_000).contains(&self.nsec) {
                return None;
            }
            Some(Duration::new(self.sec as u64, self.nsec as u32))
        }

        fn to_libc(self) -> libc::timespec {
            libc::timespec {
                tv_sec: self.sec,
                tv_nsec: self.nsec,
            }
        }
    }

    #[derive(Copy, Clone, Debug, Default)]
    pub struct Event {
        pub ident: usize,
        pub filter: i16,
        pub flags: u16,
        pub fflags: u32,
        pub data: isize,
        pub udata: usize,
    }

    impl Event {
        pub fn to_libc(self) -> libc::kevent {
            // Field widths and optional `ext` differ across BSDs; assign
            // rather than using a struct literal.
            let mut ev: libc::kevent = unsafe { core::mem::zeroed() };
            ev.ident = self.ident as _;
            ev.filter = self.filter as _;
            ev.flags = self.flags as _;
            ev.fflags = self.fflags as _;
            ev.data = self.data as _;
            ev.udata = self.udata as *mut libc::c_void;
            ev
        }

        #[allow(clippy::unnecessary_cast)]
        pub fn from_libc(e: libc::kevent) -> Self {
            Self {
                ident: e.ident as usize,
                filter: e.filter as i16,
                flags: e.flags as u16,
                fflags: e.fflags as u32,
                data: e.data as isize,
                udata: e.udata as usize,
            }
        }
    }

    static OPEN: Mutex<Vec<Weak<AtomicI32>>> = Mutex::new(Vec::new());

    fn register_open(cell: &Arc<AtomicI32>) {
        let mut open = OPEN.lock();
        open.retain(|w| w.strong_count() > 0);
        open.push(Arc::downgrade(cell));
    }

    pub fn create() -> io::Result<Arc<AtomicI32>> {
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        if let Err(err) = crate::posix::set_inheritable(borrowed, false) {
            let _ = unsafe { libc::close(fd) };
            return Err(err);
        }
        let cell = Arc::new(AtomicI32::new(fd));
        register_open(&cell);
        Ok(cell)
    }

    pub fn from_fd(fd: i32) -> Arc<AtomicI32> {
        let cell = Arc::new(AtomicI32::new(fd));
        register_open(&cell);
        cell
    }

    pub fn close(cell: &AtomicI32) -> io::Result<()> {
        let fd = cell.swap(-1, Ordering::SeqCst);
        if fd < 0 {
            return Ok(());
        }
        let ret = unsafe { libc::close(fd) };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn fd(cell: &AtomicI32) -> i32 {
        cell.load(Ordering::SeqCst)
    }

    pub fn kevent(
        kq: i32,
        changelist: &[Event],
        eventlist: &mut [Event],
        timeout: Option<&Timespec>,
    ) -> io::Result<usize> {
        let chl: Vec<libc::kevent> = changelist.iter().copied().map(Event::to_libc).collect();
        let mut evl = vec![unsafe { core::mem::zeroed() }; eventlist.len()];
        let timeout = timeout.map(|t| t.to_libc());
        let timeout = timeout.as_ref().map_or(core::ptr::null(), |t| t);
        let ret = unsafe {
            libc::kevent(
                kq,
                chl.as_ptr(),
                chl.len() as _,
                evl.as_mut_ptr(),
                evl.len() as _,
                timeout,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        let n = ret as usize;
        for (dst, src) in eventlist.iter_mut().zip(evl.into_iter().take(n)) {
            *dst = Event::from_libc(src);
        }
        Ok(n)
    }

    pub fn mark_closed_after_fork() {
        // After fork only this thread exists. If the parent held OPEN,
        // the child's copy stays locked until it is released.
        if OPEN.try_lock().is_none() {
            unsafe { OPEN.force_unlock() };
        }
        let mut open = OPEN.lock();
        for weak in open.drain(..) {
            if let Some(cell) = weak.upgrade() {
                cell.store(-1, Ordering::SeqCst);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_to_timeval_uses_micros() {
        let tv = duration_to_timeval(Duration::from_micros(1_500_250));
        assert_eq!(tv.tv_sec as u64, 1);
        assert_eq!(tv.tv_usec as u32, 500_250);
    }

    #[cfg(unix)]
    #[test]
    fn poll_timeout_rounds_fractional_millis_up() {
        assert_eq!(duration_as_millis_ceiling(Duration::ZERO), Some(0));
        assert_eq!(
            duration_as_millis_ceiling(Duration::from_micros(100)),
            Some(1)
        );
        assert_eq!(
            duration_as_millis_ceiling(Duration::from_millis(1)),
            Some(1)
        );
        assert_eq!(
            duration_as_millis_ceiling(Duration::from_micros(1100)),
            Some(2)
        );
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
    ))]
    #[test]
    fn timespec_carries_nanosecond_overflow() {
        let ts = kqueue::Timespec::from_secs(0.999_999_999_9).unwrap();
        assert_eq!(ts.sec, 1);
        assert!(ts.nsec < 1_000_000_000);
        assert!(kqueue::Timespec::from_secs(f64::INFINITY).is_none());
        assert!(kqueue::Timespec::from_secs(f64::NAN).is_none());
    }
}
