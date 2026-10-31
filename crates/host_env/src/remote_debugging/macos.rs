use super::{ReadBudget, RemoteRead, has_cookie, invalid_data, object_file, stops_probe};
use core::mem::size_of;
use mach2::{
    kern_return::{
        KERN_INVALID_ADDRESS, KERN_NO_ACCESS, KERN_PROTECTION_FAILURE, KERN_RESOURCE_SHORTAGE,
        KERN_SUCCESS, kern_return_t,
    },
    mach_port::mach_port_deallocate,
    port::{MACH_PORT_DEAD, MACH_PORT_NULL, mach_port_t},
    traps::{mach_task_self, task_for_pid},
    vm::{mach_vm_read_overwrite, mach_vm_region},
    vm_prot::{VM_PROT_EXECUTE, VM_PROT_READ},
    vm_region::{VM_REGION_BASIC_INFO_64, vm_region_basic_info_64},
};
use object::BinaryFormat;
use std::{ffi::OsStr, io, os::unix::ffi::OsStrExt, path::Path};

const MAX_FAT_ARCHES: usize = 64;

struct OwnedPort(mach_port_t);

impl Drop for OwnedPort {
    fn drop(&mut self) {
        if self.0 != MACH_PORT_NULL && self.0 != MACH_PORT_DEAD {
            // task_for_pid and mach_vm_region each return an owned send right.
            // mach_task_self is borrowed and must not itself be deallocated.
            unsafe { mach_port_deallocate(mach_task_self(), self.0) };
        }
    }
}

struct Process {
    task: OwnedPort,
    budget: ReadBudget,
}

fn mach_error(code: kern_return_t) -> io::Error {
    // Mach error codes carry the detailed code in their low fourteen bits.
    let kind = match code & 0x3fff {
        KERN_PROTECTION_FAILURE | KERN_NO_ACCESS => io::ErrorKind::PermissionDenied,
        KERN_RESOURCE_SHORTAGE => io::ErrorKind::OutOfMemory,
        KERN_INVALID_ADDRESS => io::ErrorKind::UnexpectedEof,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, format!("Mach runtime probe failed: {code}"))
}

impl RemoteRead for Process {
    fn read_exact(&mut self, address: u64, destination: &mut [u8]) -> io::Result<()> {
        if destination.is_empty() {
            return Ok(());
        }
        address
            .checked_add(destination.len() as u64 - 1)
            .ok_or_else(|| invalid_data("remote read address overflow"))?;
        self.budget.consume(destination.len())?;
        let mut count = 0;
        // Only the local destination is a dereferenceable pointer. The remote
        // address remains an integer interpreted by the kernel.
        let result = unsafe {
            mach_vm_read_overwrite(
                self.task.0,
                address,
                destination.len() as u64,
                destination.as_mut_ptr() as usize as u64,
                &mut count,
            )
        };
        if result != KERN_SUCCESS {
            return Err(mach_error(result));
        }
        if count != destination.len() as u64 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(())
    }
}

fn word<const N: usize>(bytes: &[u8], offset: usize) -> io::Result<[u8; N]> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| invalid_data("Mach-O field offset overflow"))?;
    bytes
        .get(offset..end)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| invalid_data("truncated Mach-O field"))
}

fn read_u32(bytes: &[u8], offset: usize, little: bool) -> io::Result<u32> {
    let value = word(bytes, offset)?;
    Ok(if little {
        u32::from_le_bytes(value)
    } else {
        u32::from_be_bytes(value)
    })
}

fn sysctl_int(name: &core::ffi::CStr) -> io::Result<i32> {
    let mut value = 0i32;
    let mut length = size_of::<i32>();
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut i32).cast(),
            &mut length,
            core::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if length != size_of::<i32>() {
        return Err(invalid_data("invalid CPU sysctl size"));
    }
    Ok(value)
}

fn mach_slice(bytes: &[u8]) -> io::Result<Option<&[u8]>> {
    let magic = u32::from_be_bytes(word(bytes, 0)?);
    let little = match magic {
        object::macho::FAT_MAGIC => false,
        object::macho::FAT_CIGAM => true,
        _ => return Ok(Some(bytes)),
    };
    let count = usize::try_from(read_u32(bytes, 4, little)?)
        .map_err(|_| invalid_data("Mach-O architecture count overflow"))?;
    if count > MAX_FAT_ARCHES {
        return Err(invalid_data("Mach-O architecture count exceeds its bound"));
    }
    let entry_size = 20;
    let table_end = count
        .checked_mul(entry_size)
        .and_then(|size| size.checked_add(8))
        .ok_or_else(|| invalid_data("Mach-O architecture table overflow"))?;
    if table_end > bytes.len() {
        return Err(invalid_data("truncated Mach-O architecture table"));
    }
    // Match the pinned CPython locator's host-CPU selection, rather than guessing
    // from the inspector's build target (which may itself run under translation).
    let cpu = sysctl_int(c"hw.cputype")? as u32;
    let capable = sysctl_int(c"hw.cpu64bit_capable")?;
    let cpu = cpu
        | if capable != 0 {
            object::macho::CPU_ARCH_ABI64
        } else {
            0
        };
    for index in 0..count {
        // The bounded, validated architecture table makes these offsets safe.
        let entry = 8 + index * entry_size;
        if read_u32(bytes, entry, little)? != cpu {
            continue;
        }
        let offset = u64::from(read_u32(bytes, entry + 8, little)?);
        let length = u64::from(read_u32(bytes, entry + 12, little)?);
        let end = offset
            .checked_add(length)
            .ok_or_else(|| invalid_data("Mach-O architecture range overflow"))?;
        let offset = usize::try_from(offset)
            .map_err(|_| invalid_data("Mach-O architecture offset overflow"))?;
        let end =
            usize::try_from(end).map_err(|_| invalid_data("Mach-O architecture end overflow"))?;
        if offset < table_end || length == 0 {
            return Err(invalid_data("invalid Mach-O architecture range"));
        }
        return bytes
            .get(offset..end)
            .map(Some)
            .ok_or_else(|| invalid_data("truncated Mach-O architecture"));
    }
    Ok(None)
}

fn candidate(process: &mut Process, path: &Path, base: u64) -> io::Result<bool> {
    let bytes = object_file::read(path, &mut process.budget)?;
    let Some(bytes) = mach_slice(&bytes)? else {
        return Ok(false);
    };
    let Some(address) = object_file::section_address(bytes, base, BinaryFormat::MachO)? else {
        return Ok(false);
    };
    has_cookie(process, address)
}

fn search(process: &mut Process, pid: i32, prefix: &[u8]) -> io::Result<bool> {
    let mut address = 0;
    loop {
        process.budget.consume(0)?;
        let requested = address;
        let mut size = 0;
        let mut info = vm_region_basic_info_64::default();
        let mut count = vm_region_basic_info_64::count();
        let mut object = MACH_PORT_NULL;
        let result = unsafe {
            mach_vm_region(
                process.task.0,
                &mut address,
                &mut size,
                VM_REGION_BASIC_INFO_64,
                (&mut info as *mut vm_region_basic_info_64).cast(),
                &mut count,
                &mut object,
            )
        };
        // Even irrelevant regions may return an owned memory-object port.
        let _object = OwnedPort(object);
        if result == KERN_INVALID_ADDRESS {
            return Ok(false);
        }
        if result != KERN_SUCCESS {
            return Err(mach_error(result));
        }
        if count < vm_region_basic_info_64::count() || address < requested || size == 0 {
            return Err(invalid_data("invalid Mach memory region"));
        }
        let next = address
            .checked_add(size)
            .ok_or_else(|| invalid_data("Mach memory region address overflow"))?;
        if info.protection & (VM_PROT_READ | VM_PROT_EXECUTE) == (VM_PROT_READ | VM_PROT_EXECUTE) {
            let mut path = [0u8; libc::MAXPATHLEN as usize + 1];
            process.budget.consume(0)?;
            let length = unsafe {
                // A zero return can describe an anonymous mapping; clear errno
                // first so an older failure cannot be mistaken for a denial.
                *libc::__error() = 0;
                libc::proc_regionfilename(
                    pid,
                    address,
                    path.as_mut_ptr().cast(),
                    libc::MAXPATHLEN as u32,
                )
            };
            if length <= 0 {
                let error = io::Error::last_os_error();
                if stops_probe(&error) {
                    return Err(error);
                }
            } else if (length as usize) < path.len()
                && let Some(end) = path.iter().position(|&byte| byte == 0)
                && end < libc::MAXPATHLEN as usize
            {
                let path = &path[..end];
                let name = path.rsplit(|&byte| byte == b'/').next().unwrap_or(path);
                if name.starts_with(prefix) {
                    match candidate(process, Path::new(OsStr::from_bytes(path)), address) {
                        Ok(true) => return Ok(true),
                        Err(error) if stops_probe(&error) => return Err(error),
                        Ok(false) | Err(_) => {}
                    }
                }
            }
        }
        address = next;
    }
}

pub(super) fn probe(pid: i32) -> io::Result<bool> {
    let mut task = MACH_PORT_NULL;
    let result = unsafe { task_for_pid(mach_task_self(), pid, &mut task) };
    let task = OwnedPort(task);
    if result != KERN_SUCCESS || task.0 == MACH_PORT_NULL || task.0 == MACH_PORT_DEAD {
        // CPython classifies every task_for_pid failure as a permission failure.
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("cannot acquire task port for PID {pid}: {result}"),
        ));
    }
    let mut process = Process {
        task,
        budget: ReadBudget::new(),
    };
    for prefix in [b"libpython".as_slice(), b"python", b"Python"] {
        match search(&mut process, pid, prefix) {
            Ok(true) => return Ok(true),
            Err(error) if stops_probe(&error) => return Err(error),
            Ok(false) | Err(_) => {}
        }
    }
    Ok(false)
}
