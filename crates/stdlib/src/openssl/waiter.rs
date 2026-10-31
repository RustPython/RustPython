use alloc::sync::{Arc, Weak};
use rustpython_host_env::{
    io as host_io,
    select::{self, POLLIN, POLLNVAL, POLLOUT, PollFd},
};
use std::{
    io,
    os::fd::{AsFd, AsRawFd, OwnedFd, RawFd},
    time::Instant,
};

/// A persistent, native-only notification for one blocked SSL operation.
#[derive(Debug)]
pub(super) struct ProgressWaiter {
    read_fd: OwnedFd,
    write_fd: OwnedFd,
}

impl ProgressWaiter {
    pub(super) fn new() -> io::Result<Self> {
        let (read_fd, write_fd) = wake_pipe()?;
        Ok(Self { read_fd, write_fd })
    }

    pub(super) fn notify(&self) {
        loop {
            match host_io::write_once(self.write_fd.as_fd().into(), &[1]) {
                Ok(_) => return,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                // A full pipe already carries a persistent notification.
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
                // Both endpoints stay owned for this entire call, so neither
                // EBADF nor EPIPE is possible without external fd corruption.
                Err(_) => return,
            }
        }
    }

    /// Re-arm under the connection guard, before the final identity check and
    /// close-observer subscription. Draining after subscribing can lose close.
    pub(super) fn drain(&self) -> io::Result<()> {
        let mut buffer = [0; 256];
        loop {
            match host_io::read_once(self.read_fd.as_fd().into(), &mut buffer) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "SSL progress notification pipe closed",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                // In particular, return EINTR so the caller can check signals.
                Err(error) => return Err(error),
            }
        }
    }

    /// Wait without a connection or socket guard. A missing socket means that
    /// write ownership permits only progress notifications to resume this call.
    /// Returns true on timeout; readiness never drains the notification pipe.
    pub(super) fn wait(
        &self,
        socket_fd: Option<RawFd>,
        write: bool,
        deadline: Option<Instant>,
    ) -> io::Result<bool> {
        if socket_fd.is_some_and(|fd| fd < 0) {
            return Err(io::Error::from_raw_os_error(libc::EBADF));
        }
        let mut fds = [
            PollFd {
                fd: self.read_fd.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            },
            PollFd {
                // poll ignores a negative fd when only progress is allowed.
                fd: socket_fd.unwrap_or(-1),
                // TLS consumes normal stream data, never MSG_OOB. Watching
                // POLLPRI could spin on urgent data that this BIO cannot read.
                events: if write { POLLOUT } else { POLLIN },
                revents: 0,
            },
        ];
        loop {
            let (timeout, capped) = match deadline {
                None => (-1, false),
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    match select::duration_as_millis_ceiling(remaining) {
                        Some(timeout) => (timeout, false),
                        None => (i32::MAX, true),
                    }
                }
            };
            // Even an expired deadline gets a zero-time poll, as in wait_fd.
            // EINTR is deliberately propagated for Python signal checking.
            match select::poll_fds(&mut fds, timeout)? {
                0 if capped => {}
                0 => return Ok(true),
                _ if fds.iter().any(|fd| fd.revents & POLLNVAL != 0) => {
                    return Err(io::Error::from_raw_os_error(libc::EBADF));
                }
                _ => return Ok(false),
            }
        }
    }
}

impl crate::socket::SocketCloseObserver for ProgressWaiter {
    fn socket_closed(&self) {
        self.notify();
    }
}

fn wake_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    cfg_select! {
        any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "linux",
            target_os = "android",
            target_os = "netbsd",
            target_os = "openbsd"
        ) => {
            rustpython_host_env::posix::pipe2(libc::O_CLOEXEC | libc::O_NONBLOCK)
        }
        target_os = "redox" => {
            use std::os::fd::FromRawFd;

            // Redox provides pipe2, but the host helper excludes this target.
            let mut fds = [-1; 2];
            // SAFETY: fds has storage for the two descriptors pipe2 writes.
            if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: successful pipe2 transfers ownership of two new fds.
            Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
        }
        _ => {
            // The host helper sets CLOEXEC and owns both fds immediately, so
            // any subsequent fcntl failure also closes both endpoints.
            let (read_fd, write_fd) = rustpython_host_env::posix::pipe()?;
            rustpython_host_env::fcntl::set_blocking(read_fd.as_fd(), false)?;
            rustpython_host_env::fcntl::set_blocking(write_fd.as_fd(), false)?;
            Ok((read_fd, write_fd))
        }
    }
}

/// Implementations are native-only and must not reenter the registry.
/// Production uses only ProgressWaiter's nonblocking pipe notification.
pub(super) trait ProgressNotify {
    fn notify(&self);
}

impl ProgressNotify for ProgressWaiter {
    fn notify(&self) {
        self.notify();
    }
}

struct Registrations<W> {
    generation: u64,
    waiters: Vec<Weak<W>>,
}

/// Ownership can be abandoned without acquiring the native SSL lock. Compare
/// generations when registering so that abandonment between predicate-check
/// and registration cannot strand a waiter.
pub(super) struct ProgressRegistry<W = ProgressWaiter> {
    state: std::sync::Mutex<Registrations<W>>,
}

impl<W> Default for ProgressRegistry<W> {
    fn default() -> Self {
        Self {
            state: std::sync::Mutex::new(Registrations {
                generation: 0,
                waiters: Vec::new(),
            }),
        }
    }
}

impl<W: ProgressNotify> ProgressRegistry<W> {
    pub(super) fn generation(&self) -> u64 {
        self.state.lock().unwrap().generation
    }

    pub(super) fn register(&self, waiter: &Arc<W>, observed: u64) {
        let mut state = self.state.lock().unwrap();
        let weak = Arc::downgrade(waiter);
        state.waiters.retain(|waiter| waiter.strong_count() != 0);
        if !state
            .waiters
            .iter()
            .any(|registered| registered.ptr_eq(&weak))
        {
            state.waiters.push(weak);
        }
        // Comparison and queuing share the notify mutex. A concurrent release
        // either changed this generation or will find this registered waiter.
        if state.generation != observed {
            waiter.notify();
        }
    }

    pub(super) fn unregister(&self, waiter: &Arc<W>) {
        let weak = Arc::downgrade(waiter);
        self.state
            .lock()
            .unwrap()
            .waiters
            .retain(|registered| !registered.ptr_eq(&weak));
    }

    pub(super) fn notify(&self) {
        let mut state = self.state.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
        state.waiters.retain(|waiter| {
            if let Some(waiter) = waiter.upgrade() {
                waiter.notify();
                true
            } else {
                false
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Counter(AtomicUsize);

    impl ProgressNotify for Counter {
        fn notify(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn release_before_registration_is_not_lost() {
        let registry = ProgressRegistry::default();
        let observed = registry.generation();
        let waiter = Arc::new(Counter::default());
        registry.notify();
        registry.register(&waiter, observed);
        assert_eq!(waiter.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn release_after_registration_notifies_and_unregister_removes_it() {
        let registry = ProgressRegistry::default();
        let waiter = Arc::new(Counter::default());
        registry.register(&waiter, registry.generation());
        assert_eq!(waiter.0.load(Ordering::Relaxed), 0);
        registry.notify();
        assert_eq!(waiter.0.load(Ordering::Relaxed), 1);
        registry.unregister(&waiter);
        registry.notify();
        assert_eq!(waiter.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn unchanged_retry_does_not_self_wake_or_duplicate_registration() {
        let registry = ProgressRegistry::default();
        let waiter = Arc::new(Counter::default());
        for _ in 0..10 {
            registry.register(&waiter, registry.generation());
        }
        assert_eq!(waiter.0.load(Ordering::Relaxed), 0);
        assert_eq!(registry.state.lock().unwrap().waiters.len(), 1);
        registry.notify();
        assert_eq!(waiter.0.load(Ordering::Relaxed), 1);
        let fresh = registry.generation();
        registry.register(&waiter, fresh);
        assert_eq!(waiter.0.load(Ordering::Relaxed), 1);
    }
}
