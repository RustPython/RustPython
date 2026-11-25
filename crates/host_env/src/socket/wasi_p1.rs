//! Standard preview1 socket imports for the host_io guest.
//! No native descriptors or ambient network creation are exposed.
#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use std::{io, mem};

const FILETYPE_SOCKET_DGRAM: u8 = 5;
const FILETYPE_SOCKET_STREAM: u8 = 6;
const FDFLAGS_NONBLOCK: u16 = 4;
const NOTSOCK: i32 = 57;
const NOTSUP: i32 = 58;

#[repr(C)]
#[derive(Default)]
struct FdStat {
    filetype: u8,
    padding0: u8,
    flags: u16,
    padding1: u32,
    rights_base: u64,
    rights_inheriting: u64,
}
#[repr(C)]
struct Iovec {
    pointer: u32,
    length: u32,
}
#[repr(C)]
#[derive(Default)]
struct Subscription {
    userdata: u64,
    tag: u8,
    padding: [u8; 7],
    payload: [u8; 32],
}
#[repr(C)]
#[derive(Default)]
struct Event {
    userdata: u64,
    error: u16,
    tag: u8,
    padding: [u8; 5],
    nbytes: u64,
    flags: u16,
    padding2: [u8; 6],
}
const _: () = assert!(mem::size_of::<FdStat>() == 24);
const _: () = assert!(mem::size_of::<Iovec>() == 8);
const _: () = assert!(mem::size_of::<Subscription>() == 48);
const _: () = assert!(mem::align_of::<Subscription>() == 8);
const _: () = assert!(mem::size_of::<Event>() == 32);

#[link(wasm_import_module = "wasi_snapshot_preview1")]
unsafe extern "C" {
    #[link_name = "fd_close"]
    fn raw_close(fd: i32) -> i32;
    #[link_name = "fd_fdstat_get"]
    fn raw_fdstat(fd: i32, result: i32) -> i32;
    #[link_name = "fd_fdstat_set_flags"]
    fn raw_set_flags(fd: i32, flags: i32) -> i32;
    #[link_name = "sock_accept"]
    fn raw_accept(fd: i32, flags: i32, result: i32) -> i32;
    #[link_name = "sock_recv"]
    fn raw_recv(
        fd: i32,
        vectors: i32,
        count: i32,
        flags: i32,
        received: i32,
        out_flags: i32,
    ) -> i32;
    #[link_name = "sock_send"]
    fn raw_send(fd: i32, vectors: i32, count: i32, flags: i32, sent: i32) -> i32;
    #[link_name = "sock_shutdown"]
    fn raw_shutdown(fd: i32, how: i32) -> i32;
    #[link_name = "clock_time_get"]
    fn raw_clock_time(clock: i32, precision: i64, result: i32) -> i32;
    #[link_name = "poll_oneoff"]
    fn raw_poll(input: i32, output: i32, count: i32, result_count: i32) -> i32;
}
fn pointer<T>(value: *const T) -> i32 {
    value as usize as i32
}
fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}
fn fdstat(fd: i32) -> io::Result<FdStat> {
    let mut result = FdStat::default();
    // SAFETY: result is a properly sized/aligned output in guest memory.
    check(unsafe { raw_fdstat(fd, pointer(&mut result)) })?;
    Ok(result)
}

/// Validate the descriptor before an owning Socket may adopt it.
pub fn socket_type(fd: i32) -> io::Result<i32> {
    let stat = fdstat(fd).map_err(|_| io::Error::from_raw_os_error(8))?;
    match stat.filetype {
        FILETYPE_SOCKET_DGRAM | FILETYPE_SOCKET_STREAM => Ok(i32::from(stat.filetype)),
        _ => Err(io::Error::from_raw_os_error(NOTSOCK)),
    }
}
pub fn set_nonblocking(fd: i32, nonblocking: bool) -> io::Result<()> {
    let stat = fdstat(fd)?;
    let flags = if nonblocking {
        stat.flags | FDFLAGS_NONBLOCK
    } else {
        stat.flags & !FDFLAGS_NONBLOCK
    };
    // SAFETY: no memory arguments. Host enforces the descriptor rights.
    check(unsafe { raw_set_flags(fd, i32::from(flags)) })
}

pub fn accept(fd: i32, nonblocking: bool) -> io::Result<i32> {
    let mut accepted = -1_i32;
    let flags = if nonblocking {
        i32::from(FDFLAGS_NONBLOCK)
    } else {
        0
    };
    // SAFETY: accepted is a valid fd-sized output. No peer address exists in p1.
    check(unsafe { raw_accept(fd, flags, pointer(&mut accepted)) })?;
    if accepted < 0 {
        // preview1 returns u32 descriptors, but Python uses a signed fd here.
        // Close the successful unsigned descriptor before rejecting its range.
        unsafe { raw_close(accepted) };
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid accepted descriptor",
        ));
    }
    Ok(accepted)
}

pub fn recv(fd: i32, buffer: &mut [u8], flags: i32) -> io::Result<usize> {
    // Match wasi-libc recv: PEEK and WAITALL are the only input flags.
    if flags & !3 != 0 {
        return Err(io::Error::from_raw_os_error(NOTSUP));
    }
    let vector = Iovec {
        pointer: buffer.as_mut_ptr() as u32,
        length: buffer.len() as u32,
    };
    let mut received = 0_u32;
    let mut output_flags = 0_u16;
    // SAFETY: vector points to the entire writable buffer; all outputs remain live.
    check(unsafe {
        raw_recv(
            fd,
            pointer(&vector),
            1,
            flags,
            pointer(&mut received),
            pointer(&mut output_flags),
        )
    })?;
    checked_count(received, buffer.len())
}
pub fn send(fd: i32, buffer: &[u8], flags: i32) -> io::Result<usize> {
    if flags != 0 {
        return Err(io::Error::from_raw_os_error(NOTSUP));
    }
    let vector = Iovec {
        pointer: buffer.as_ptr() as u32,
        length: buffer.len() as u32,
    };
    let mut sent = 0_u32;
    // SAFETY: vector borrows readable buffer; sent is a writable result.
    check(unsafe { raw_send(fd, pointer(&vector), 1, 0, pointer(&mut sent)) })?;
    checked_count(sent, buffer.len())
}
fn checked_count(count: u32, maximum: usize) -> io::Result<usize> {
    let count = count as usize;
    if count > maximum {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "socket count exceeds buffer",
        ))
    } else {
        Ok(count)
    }
}
pub fn shutdown(fd: i32, how: i32) -> io::Result<()> {
    // p1 libc SHUT_* values are exactly the WASI shutdown bitflags, not Unix 0/1/2.
    if !(1..=3).contains(&how) {
        return Err(io::Error::from_raw_os_error(28));
    }
    check(unsafe { raw_shutdown(fd, how) })
}

/// Return true when the requested descriptor is ready, false only for the
/// monotonic deadline. Per-event errors are propagated, not treated as readiness.
pub fn wait(fd: i32, write: bool, timeout: Option<std::time::Duration>) -> io::Result<bool> {
    let tag = if write { 2 } else { 1 };
    let mut subscriptions: [Subscription; 2] = Default::default();
    subscriptions[0].tag = tag;
    subscriptions[0].payload[..4].copy_from_slice(&fd.to_le_bytes());
    let count = if let Some(timeout) = timeout {
        subscriptions[1].userdata = 1;
        // Clock id MONOTONIC=1, relative deadline flag=0. Clamp rather than wrap.
        subscriptions[1].payload[..4].copy_from_slice(&1_u32.to_le_bytes());
        let nanos = timeout.as_nanos().min(u128::from(u64::MAX)) as u64;
        subscriptions[1].payload[8..16].copy_from_slice(&nanos.to_le_bytes());
        2
    } else {
        1
    };
    let mut events: [Event; 2] = Default::default();
    let mut event_count = 0_u32;
    // SAFETY: each array has room for count items and event_count is a valid output.
    check(unsafe {
        raw_poll(
            pointer(subscriptions.as_ptr()),
            pointer(events.as_mut_ptr()),
            count,
            pointer(&mut event_count),
        )
    })?;
    if event_count == 0 || event_count > count as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid poll event count",
        ));
    }
    let mut timed_out = false;
    let mut ready = false;
    for event in &events[..event_count as usize] {
        check(i32::from(event.error))?;
        match (event.userdata, event.tag) {
            (0, event_tag) if event_tag == tag => ready = true,
            (1, 0) if timeout.is_some() => timed_out = true,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected poll event",
                ));
            }
        }
    }
    if ready {
        Ok(true)
    } else if timed_out {
        Ok(false)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "poll returned no matching event",
        ))
    }
}

// Rust's unknown-WASM std backend does not classify raw WASI errno values.
// The common socket retry loop must call these backend predicates rather
// than losing errno by wrapping errors in io::Error::new.
pub fn is_interrupted(error: &io::Error) -> bool {
    error.raw_os_error() == Some(27) || error.kind() == io::ErrorKind::Interrupted
}
pub fn would_block(error: &io::Error) -> bool {
    error.raw_os_error() == Some(6) || error.kind() == io::ErrorKind::WouldBlock
}

/// One absolute host deadline, reusable across EINTR and spurious readiness.
/// Never uses unknown-WASM's unsupported std::time::Instant.
#[derive(Clone, Copy)]
pub struct Deadline(u64);
fn monotonic_now() -> io::Result<u64> {
    let mut now = 0_u64;
    // MONOTONIC = 1; output has the timestamp ABI's alignment and width.
    check(unsafe { raw_clock_time(1, 1, pointer(&mut now)) })?;
    Ok(now)
}
impl Deadline {
    pub fn new(timeout: std::time::Duration) -> io::Result<Self> {
        let nanos = u64::try_from(timeout.as_nanos())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "timeout too large"))?;
        let end = monotonic_now()?
            .checked_add(nanos)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "deadline overflow"))?;
        Ok(Self(end))
    }
    pub fn remaining(self) -> io::Result<std::time::Duration> {
        Ok(std::time::Duration::from_nanos(
            self.0.saturating_sub(monotonic_now()?),
        ))
    }
}

/// Owns one preview1 guest descriptor, not a host-native descriptor.
/// The resource table must pin its host object during an in-flight operation.
pub struct OwnedSocket(i32);
impl OwnedSocket {
    /// Validate and adopt a caller-supplied descriptor only on success.
    ///
    /// # Safety
    /// The caller must transfer unique ownership on success and must not close
    /// or reuse this descriptor while the returned owner is alive.
    pub unsafe fn from_raw(fd: i32) -> io::Result<Self> {
        socket_type(fd)?;
        Ok(Self(fd))
    }
    pub fn as_raw(&self) -> i32 {
        self.0
    }
    pub fn into_raw(self) -> i32 {
        let this = mem::ManuallyDrop::new(self);
        this.0
    }
    pub fn accept(&self, nonblocking: bool) -> io::Result<Self> {
        // The host creates a new descriptor. Own it immediately, including
        // while checking its type, so any validation error closes it.
        let accepted = Self(accept(self.0, nonblocking)?);
        socket_type(accepted.0)?;
        Ok(accepted)
    }
}
impl Drop for OwnedSocket {
    fn drop(&mut self) {
        // Do not retry close: a guest descriptor may already have been reused.
        // SAFETY: this object owns the descriptor until this single close.
        unsafe { raw_close(self.0) };
    }
}
