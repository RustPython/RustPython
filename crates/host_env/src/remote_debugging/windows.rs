use super::{ReadBudget, RemoteRead, has_cookie, invalid_data, object_file, stops_probe};
use core::mem::size_of;
use object::BinaryFormat;
use std::{ffi::OsString, io, os::windows::ffi::OsStringExt, path::PathBuf};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_BAD_LENGTH, ERROR_NO_MORE_FILES, GetLastError, HANDLE,
        INVALID_HANDLE_VALUE,
    },
    System::{
        Diagnostics::{
            Debug::ReadProcessMemory,
            ToolHelp::{
                CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW,
                TH32CS_SNAPMODULE,
            },
        },
        Threading::{
            OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SUSPEND_RESUME, PROCESS_VM_OPERATION,
            PROCESS_VM_READ, PROCESS_VM_WRITE,
        },
    },
};

const MAX_SNAPSHOT_ATTEMPTS: usize = 16;
const MAX_MODULES: usize = 16_384;

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // Only successfully acquired process and snapshot handles are owned here.
        unsafe { CloseHandle(self.0) };
    }
}

struct Process {
    handle: OwnedHandle,
    budget: ReadBudget,
}

impl RemoteRead for Process {
    fn read_exact(&mut self, address: u64, mut destination: &mut [u8]) -> io::Result<()> {
        if destination.is_empty() {
            return Ok(());
        }
        let end = address
            .checked_add(destination.len() as u64 - 1)
            .ok_or_else(|| invalid_data("remote read address overflow"))?;
        usize::try_from(end).map_err(|_| invalid_data("remote address exceeds pointer width"))?;
        let mut address = usize::try_from(address)
            .map_err(|_| invalid_data("remote address exceeds pointer width"))?;
        while !destination.is_empty() {
            self.budget.consume(destination.len())?;
            let mut count = 0;
            // The address is passed to the OS, never dereferenced in this process.
            let success = unsafe {
                ReadProcessMemory(
                    self.handle.0,
                    address as *const _,
                    destination.as_mut_ptr().cast(),
                    destination.len(),
                    &mut count,
                )
            };
            if success == 0 {
                // In particular, an ERROR_PARTIAL_COPY failure is not a successful
                // short read. CPython only continues after successful reads.
                return Err(io::Error::last_os_error());
            }
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            if count > destination.len() {
                return Err(invalid_data("remote read exceeded its destination"));
            }
            destination = &mut destination[count..];
            if !destination.is_empty() {
                address = address
                    .checked_add(count)
                    .ok_or_else(|| invalid_data("remote read address overflow"))?;
            }
        }
        Ok(())
    }
}

fn snapshot(pid: u32, budget: &mut ReadBudget) -> io::Result<OwnedHandle> {
    for _ in 0..MAX_SNAPSHOT_ATTEMPTS {
        budget.consume(0)?;
        // Keep CPython's module selection flags, including its cross-bitness limits.
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, pid) };
        if handle != INVALID_HANDLE_VALUE {
            return Ok(OwnedHandle(handle));
        }
        let error = unsafe { GetLastError() };
        if error != ERROR_BAD_LENGTH {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
    }
    Err(io::Error::from_raw_os_error(ERROR_BAD_LENGTH as i32))
}

fn terminated(value: &[u16]) -> io::Result<&[u16]> {
    let end = value
        .iter()
        .position(|&unit| unit == 0)
        .ok_or_else(|| invalid_data("unterminated module name or path"))?;
    Ok(&value[..end])
}

fn candidate(process: &mut Process, module: &MODULEENTRY32W) -> io::Result<bool> {
    const PYTHON: &[u16] = &[112, 121, 116, 104, 111, 110];
    let name = terminated(&module.szModule)?;
    if !name.windows(PYTHON.len()).any(|part| part == PYTHON) {
        return Ok(false);
    }
    let path = PathBuf::from(OsString::from_wide(terminated(&module.szExePath)?));
    let bytes = object_file::read(&path, &mut process.budget)?;
    let base = module.modBaseAddr as usize as u64;
    let Some(address) = object_file::section_address(&bytes, base, BinaryFormat::Pe)? else {
        return Ok(false);
    };
    has_cookie(process, address)
}

pub(super) fn probe(pid: i32) -> io::Result<bool> {
    // CPython requests these rights even for this read-only operation. Reducing
    // them could make detection succeed where CPython's handle initialization fails.
    // The DWORD conversion also preserves the Windows API's signed-PID behavior.
    let handle = unsafe {
        OpenProcess(
            PROCESS_VM_READ
                | PROCESS_VM_WRITE
                | PROCESS_VM_OPERATION
                | PROCESS_QUERY_INFORMATION
                | PROCESS_SUSPEND_RESUME,
            0,
            pid as u32,
        )
    };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut process = Process {
        handle: OwnedHandle(handle),
        budget: ReadBudget::new(),
    };
    let snapshot = snapshot(pid as u32, &mut process.budget)?;
    let mut module = MODULEENTRY32W {
        dwSize: size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    process.budget.consume(0)?;
    let mut success = unsafe { Module32FirstW(snapshot.0, &mut module) };
    for _ in 0..MAX_MODULES {
        if success == 0 {
            let error = unsafe { GetLastError() };
            return if error == ERROR_NO_MORE_FILES {
                Ok(false)
            } else {
                Err(io::Error::from_raw_os_error(error as i32))
            };
        }
        match candidate(&mut process, &module) {
            Ok(true) => return Ok(true),
            Err(error) if stops_probe(&error) => return Err(error),
            Ok(false) | Err(_) => {}
        }
        process.budget.consume(0)?;
        module.dwSize = size_of::<MODULEENTRY32W>() as u32;
        success = unsafe { Module32NextW(snapshot.0, &mut module) };
    }
    Err(io::Error::new(
        io::ErrorKind::OutOfMemory,
        "runtime probe module budget exhausted",
    ))
}
