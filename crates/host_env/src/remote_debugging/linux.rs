use super::{COOKIE, ReadBudget, RemoteRead, has_cookie, invalid_data, object_file, stops_probe};
use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{self, Read},
    os::unix::{ffi::OsStrExt, fs::FileExt},
    path::Path,
};

const MAX_MAPS_BYTES: u64 = 8 * 1024 * 1024;
const SCAN_CHUNK: usize = 1024 * 1024;

struct Process {
    pid: i32,
    memory: Option<File>,
    budget: ReadBudget,
}

impl RemoteRead for Process {
    fn read_exact(&mut self, address: u64, destination: &mut [u8]) -> io::Result<()> {
        let address = usize::try_from(address)
            .map_err(|_| invalid_data("remote address exceeds native pointer width"))?;
        address
            .checked_add(destination.len())
            .ok_or_else(|| invalid_data("remote read address overflow"))?;
        let mut completed = 0;
        while completed < destination.len() {
            let remaining = &mut destination[completed..];
            self.budget.consume(remaining.len())?;
            let current = address + completed;
            let count = if let Some(memory) = &self.memory {
                memory.read_at(remaining, current as u64)?
            } else {
                let local = libc::iovec {
                    iov_base: remaining.as_mut_ptr().cast(),
                    iov_len: remaining.len(),
                };
                let remote = libc::iovec {
                    iov_base: current as *mut libc::c_void,
                    iov_len: remaining.len(),
                };
                // SAFETY: local names the writable destination slice. The
                // remote pointer is only an address for the kernel, never a
                // pointer dereferenced by this process.
                let result = unsafe { libc::process_vm_readv(self.pid, &local, 1, &remote, 1, 0) };
                if result < 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::ENOSYS) {
                        // Match CPython's fallback only for a missing syscall.
                        // Permission denials must not select another route.
                        self.memory = Some(
                            OpenOptions::new()
                                .read(true)
                                .write(true)
                                .open(format!("/proc/{}/mem", self.pid))?,
                        );
                        continue;
                    }
                    return Err(error);
                }
                result as usize
            };
            if count == 0 || count > remaining.len() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            completed += count;
        }
        Ok(())
    }
}

struct Mapping<'a> {
    start: u64,
    end: u64,
    writable: bool,
    path: &'a [u8],
}

fn token<'a>(remaining: &mut &'a [u8]) -> Option<&'a [u8]> {
    let source = (*remaining).trim_ascii_start();
    if source.is_empty() {
        return None;
    }
    let end = source
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(source.len());
    let (value, rest) = source.split_at(end);
    *remaining = rest;
    Some(value)
}

fn mapping(line: &[u8]) -> Option<Mapping<'_>> {
    let mut fields = line;
    let range = token(&mut fields)?;
    let permissions = token(&mut fields)?;
    for _ in 0..3 {
        token(&mut fields)?;
    }
    let path = fields.trim_ascii_start();
    if path.is_empty() || (path.starts_with(b"[") && path.ends_with(b"]")) {
        return None;
    }
    let dash = range.iter().position(|byte| *byte == b'-')?;
    let parse = |bytes| u64::from_str_radix(core::str::from_utf8(bytes).ok()?, 16).ok();
    let start = parse(&range[..dash])?;
    let end = parse(&range[dash + 1..])?;
    if start == 0 || end <= start {
        return None;
    }
    Some(Mapping {
        start,
        end,
        writable: permissions.starts_with(b"rw"),
        path,
    })
}

fn scan_deleted(process: &mut Process, mapping: &Mapping<'_>) -> io::Result<Option<u64>> {
    let mut buffer = vec![0; SCAN_CHUNK];
    let mut address = mapping.start;
    while address < mapping.end {
        let wanted = usize::try_from((mapping.end - address).min(SCAN_CHUNK as u64))
            .map_err(|_| invalid_data("mapping scan size overflow"))?;
        match process.read_exact(address, &mut buffer[..wanted]) {
            Ok(()) => {
                if let Some(offset) = buffer[..wanted]
                    .windows(COOKIE.len())
                    .position(|bytes| bytes == COOKIE)
                {
                    return Ok(Some(address + offset as u64));
                }
            }
            Err(error) if stops_probe(&error) => return Err(error),
            Err(_) => {}
        }
        let overlap = COOKIE.len() - 1;
        if wanted <= overlap {
            break;
        }
        address = address
            .checked_add((wanted - overlap) as u64)
            .ok_or_else(|| invalid_data("mapping scan address overflow"))?;
    }
    Ok(None)
}

pub(super) fn probe(pid: i32) -> io::Result<bool> {
    let mut maps = Vec::new();
    File::open(format!("/proc/{pid}/maps"))?
        .take(MAX_MAPS_BYTES + 1)
        .read_to_end(&mut maps)?;
    if maps.len() as u64 > MAX_MAPS_BYTES {
        return Err(invalid_data("process map exceeds its size bound"));
    }
    let mut process = Process {
        pid,
        memory: None,
        budget: ReadBudget::new(),
    };
    for line in maps.split(|byte| *byte == b'\n') {
        process.budget.consume(0)?;
        let Some(mapping) = mapping(line) else {
            continue;
        };
        let path = Path::new(OsStr::from_bytes(mapping.path));
        let Some(name) = path.file_name() else {
            continue;
        };
        if !name.as_bytes().windows(6).any(|bytes| bytes == b"python") {
            continue;
        }
        let candidate = if mapping.path.ends_with(b" (deleted)") {
            if !mapping.writable {
                continue;
            }
            scan_deleted(&mut process, &mapping)
        } else {
            object_file::read(path, &mut process.budget).and_then(|bytes| {
                object_file::section_address(&bytes, mapping.start, object::BinaryFormat::Elf)
            })
        };
        match candidate {
            Ok(Some(address)) if has_cookie(&mut process, address)? => return Ok(true),
            Err(error) if stops_probe(&error) => return Err(error),
            _ => {}
        }
    }
    Ok(false)
}
