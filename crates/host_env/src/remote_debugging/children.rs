//! Process-tree snapshots, independent of remote stack-unwinding support.
use std::io;

#[derive(Debug)]
pub enum Error {
    Os(io::Error),
    Proc {
        error: io::Error,
        operation: Option<&'static str>,
    },
    Memory,
    Unsupported,
    Message(&'static str),
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Os(error)
    }
}

fn push<T>(items: &mut Vec<T>, item: T) -> Result<(), Error> {
    items.try_reserve(1).map_err(|_| Error::Memory)?;
    items.push(item);
    Ok(())
}

pub(super) fn enumerate(pid: i32, recursive: bool) -> Result<Vec<i32>, Error> {
    let processes = snapshot()?;
    let mut result = Vec::new();
    let mut pending = Vec::new();
    push(&mut pending, pid)?;
    let mut next = 0;
    while let Some(&parent) = pending.get(next) {
        next += 1;
        // Preserve the platform snapshot order, like CPython's BFS.
        for &(child, ppid) in &processes {
            if ppid == parent && !result.contains(&child) {
                push(&mut result, child)?;
                if recursive {
                    push(&mut pending, child)?;
                }
            }
        }
        if !recursive {
            break;
        }
    }
    Ok(result)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn snapshot() -> Result<Vec<(i32, i32)>, Error> {
    use core::{ffi::CStr, ptr::NonNull};
    use std::{fs::File, io::Read};

    struct Directory(Option<NonNull<libc::DIR>>);
    impl Directory {
        fn close(&mut self) -> io::Result<()> {
            if let Some(directory) = self.0.take() {
                // SAFETY: this uniquely owned handle came from opendir and is
                // taken before closing, including on a failed close.
                if unsafe { libc::closedir(directory.as_ptr()) } != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = self.close();
        }
    }

    fn parent(pid: i32) -> Option<i32> {
        let mut file = File::open(format!("/proc/{pid}/stat")).ok()?;
        // CPython reads at most 2047 bytes of stat, once.
        let mut bytes = [0_u8; 2047];
        let count = file.read(&mut bytes).ok()?;
        let bytes = &bytes[..count];
        let end = bytes.iter().rposition(|&byte| byte == b')')?;
        let tail = bytes.get(end + 2..)?;
        let mut fields = tail
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|s| !s.is_empty());
        fields.next()?; // process state
        let ppid = core::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
        (ppid >= 0).then_some(ppid)
    }

    // SAFETY: the path is a static NUL-terminated string.
    let pointer =
        NonNull::new(unsafe { libc::opendir(c"/proc".as_ptr()) }).ok_or_else(|| Error::Proc {
            error: io::Error::last_os_error(),
            operation: None,
        })?;
    let mut directory = Directory(Some(pointer));
    let mut processes = Vec::new();
    loop {
        crate::os::clear_errno();
        // SAFETY: pointer remains open for this loop; the entry is consumed
        // before the next readdir call, with no concurrent access to the DIR.
        let entry = unsafe { libc::readdir(pointer.as_ptr()) };
        if entry.is_null() {
            let errno = crate::os::get_errno();
            if errno != 0 {
                return Err(Error::Proc {
                    error: io::Error::from_raw_os_error(errno),
                    operation: Some("read"),
                });
            }
            break;
        }
        // SAFETY: readdir supplies a NUL-terminated d_name valid until its next call.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if !matches!(name.first(), Some(b'1'..=b'9')) || !name.iter().all(u8::is_ascii_digit) {
            continue;
        }
        let Some(pid) = core::str::from_utf8(name)
            .ok()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        if let Some(ppid) = parent(pid) {
            push(&mut processes, (pid, ppid))?;
        }
    }
    directory.close().map_err(|error| Error::Proc {
        error,
        operation: Some("close"),
    })?;
    Ok(processes)
}

#[cfg(target_os = "macos")]
fn snapshot() -> Result<Vec<(i32, i32)>, Error> {
    use core::{mem::size_of, ptr};

    // Apple's <sys/proc_info.h>; libc exposes the functions but not this constant.
    const PROC_ALL_PIDS: u32 = 1;
    // SAFETY: a null buffer and zero size query the required byte count.
    let count = unsafe { libc::proc_listpids(PROC_ALL_PIDS, 0, ptr::null_mut(), 0) };
    if count <= 0 {
        return Err(Error::Message("Failed to get process count"));
    }
    let capacity = (count as usize / size_of::<libc::pid_t>())
        .checked_add(64)
        .ok_or(Error::Memory)?;
    let bytes = capacity
        .checked_mul(size_of::<libc::pid_t>())
        .and_then(|size| i32::try_from(size).ok())
        .ok_or(Error::Memory)?;
    let mut pids = Vec::<libc::pid_t>::new();
    pids.try_reserve_exact(capacity)
        .map_err(|_| Error::Memory)?;
    pids.resize(capacity, 0);
    // SAFETY: the initialized Vec provides exactly the passed writable byte size.
    let actual = unsafe { libc::proc_listpids(PROC_ALL_PIDS, 0, pids.as_mut_ptr().cast(), bytes) };
    if actual <= 0 {
        return Err(Error::Message("Failed to list PIDs"));
    }
    if actual > bytes {
        return Err(Error::Message("process list exceeds buffer"));
    }
    let mut processes = Vec::new();
    for &pid in &pids[..actual as usize / size_of::<libc::pid_t>()] {
        // SAFETY: proc_bsdinfo is a C structure of integer and byte-array fields.
        let mut info: libc::proc_bsdinfo = unsafe { core::mem::zeroed() };
        // SAFETY: info is a live, aligned buffer of the supplied size.
        let got = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if got == size_of::<libc::proc_bsdinfo>() as i32 {
            push(&mut processes, (pid, info.pbi_ppid as i32))?;
        }
    }
    Ok(processes)
}

#[cfg(windows)]
fn snapshot() -> Result<Vec<(i32, i32)>, Error> {
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_NO_MORE_FILES, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
        },
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };
    struct Snapshot(HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            // SAFETY: only a valid uniquely owned snapshot is stored here.
            unsafe { CloseHandle(self.0) };
        }
    }
    // SAFETY: snapshot enumeration needs no pointer arguments or process mutation.
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error().into());
    }
    let snapshot = Snapshot(handle);
    // SAFETY: PROCESSENTRY32W is a C structure of integer and character-array fields.
    let mut entry: PROCESSENTRY32W = unsafe { core::mem::zeroed() };
    entry.dwSize = core::mem::size_of::<PROCESSENTRY32W>() as u32;
    // SAFETY: the handle is valid and entry is an initialized full-sized buffer.
    if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut processes = Vec::new();
    loop {
        push(
            &mut processes,
            (entry.th32ProcessID as i32, entry.th32ParentProcessID as i32),
        )?;
        // SAFETY: the same owned snapshot and live buffer remain valid.
        if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
            // Read the error before Drop closes the snapshot.
            let error = unsafe { GetLastError() };
            if error != ERROR_NO_MORE_FILES {
                return Err(io::Error::from_raw_os_error(error as i32).into());
            }
            break;
        }
    }
    Ok(processes)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    windows
)))]
fn snapshot() -> Result<Vec<(i32, i32)>, Error> {
    Err(Error::Unsupported)
}
