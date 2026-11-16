// cspell:ignore iovecs iovs nwritten fdflags fdstat
use super::*;

/// A guest's linear memory. Implementations must report bounds failures as `FAULT`.
/// Memory must not shrink during a call. The adapter validates every buffer before I/O.
pub trait GuestMemory {
    /// Validate that the guest has supplied a memory before interpreting pointers.
    fn validate(&self) -> Result<(), u16> {
        Ok(())
    }
    fn size(&self) -> u64;
    fn read(&self, offset: u32, bytes: &mut [u8]) -> Result<(), u16>;
    fn write(&self, offset: u32, bytes: &[u8]) -> Result<(), u16>;
}

fn abi(errno: Result<(), u16>) -> i32 {
    i32::from(errno.err().unwrap_or(SUCCESS))
}

fn guest_u32(ptr: i32) -> Result<u32, u16> {
    u32::try_from(ptr).map_err(|_| FAULT)
}

pub(crate) fn check_range(size: u64, ptr: u32, len: u32) -> Result<(), u16> {
    match u64::from(ptr).checked_add(u64::from(len)) {
        Some(end) if end <= size => Ok(()),
        _ => Err(FAULT),
    }
}

fn mem_read(view: &impl GuestMemory, ptr: u32, buf: &mut [u8]) -> Result<(), u16> {
    check_range(view.size(), ptr, buf.len() as u32)?;
    view.read(ptr, buf)
}

fn mem_write(view: &impl GuestMemory, ptr: u32, buf: &[u8]) -> Result<(), u16> {
    check_range(view.size(), ptr, buf.len() as u32)?;
    view.write(ptr, buf)
}

fn mem_read_u32(view: &impl GuestMemory, ptr: u32) -> Result<u32, u16> {
    let mut bytes = [0u8; 4];
    mem_read(view, ptr, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn mem_write_u32(view: &impl GuestMemory, ptr: u32, value: u32) -> Result<(), u16> {
    mem_write(view, ptr, &value.to_le_bytes())
}

fn mem_write_u64(view: &impl GuestMemory, ptr: u32, value: u64) -> Result<(), u16> {
    mem_write(view, ptr, &value.to_le_bytes())
}

fn read_iovecs(view: &impl GuestMemory, iovs: i32, iovs_len: i32) -> Result<Vec<(u32, u32)>, u16> {
    let n = u32::try_from(iovs_len).map_err(|_| INVAL)?;
    let iovs = guest_u32(iovs)?;
    let bytes = n.checked_mul(8).ok_or(INVAL)?;
    check_range(view.size(), iovs, bytes)?;
    let mut out = Vec::new();
    for i in 0..n {
        let base = iovs + i * 8;
        let buf = mem_read_u32(view, base)?;
        let len = mem_read_u32(view, base + 4)?;
        if len != 0 {
            check_range(view.size(), buf, len)?;
        }
        out.push((buf, len));
    }
    Ok(out)
}

pub fn fd_read(
    table: &mut Table,
    view: &impl GuestMemory,
    fd: i32,
    iovs: i32,
    iovs_len: i32,
    nread: i32,
) -> i32 {
    abi((|| {
        view.validate()?;
        let nread = guest_u32(nread)?;
        check_range(view.size(), nread, 4)?;
        let iovecs = read_iovecs(view, iovs, iovs_len)?;
        let mut storage: Vec<Vec<u8>> = iovecs
            .iter()
            .map(|&(_, len)| vec![0u8; len as usize])
            .collect();
        let mut slices: Vec<&mut [u8]> = storage.iter_mut().map(|buf| buf.as_mut_slice()).collect();
        let n = table.fd_read(fd, &mut slices)?;
        if n > MAX_TRANSFER {
            return Err(IO);
        }
        let mut remaining = n;
        for ((ptr, len), buf) in iovecs.iter().zip(storage.iter()) {
            if remaining == 0 {
                break;
            }
            let take = remaining.min(*len as usize);
            mem_write(view, *ptr, &buf[..take])?;
            remaining -= take;
        }
        let written = u32::try_from(n).map_err(|_| OVERFLOW)?;
        mem_write_u32(view, nread, written)
    })())
}

pub fn fd_write(
    table: &mut Table,
    view: &impl GuestMemory,
    fd: i32,
    iovs: i32,
    iovs_len: i32,
    nwritten: i32,
) -> i32 {
    abi((|| {
        view.validate()?;
        let nwritten = guest_u32(nwritten)?;
        check_range(view.size(), nwritten, 4)?;
        let iovecs = read_iovecs(view, iovs, iovs_len)?;
        let mut storage = Vec::new();
        for (ptr, len) in &iovecs {
            let mut buf = vec![0u8; *len as usize];
            if *len != 0 {
                mem_read(view, *ptr, &mut buf)?;
            }
            storage.push(buf);
        }
        let slices: Vec<&[u8]> = storage.iter().map(|buf| buf.as_slice()).collect();
        let n = table.fd_write(fd, &slices)?;
        if n > MAX_TRANSFER {
            return Err(IO);
        }
        let written = u32::try_from(n).map_err(|_| OVERFLOW)?;
        mem_write_u32(view, nwritten, written)
    })())
}

pub fn fd_seek(
    table: &mut Table,
    view: &impl GuestMemory,
    fd: i32,
    offset: i64,
    whence: i32,
    newoffset: i32,
) -> i32 {
    abi((|| {
        view.validate()?;
        let newoffset = guest_u32(newoffset)?;
        check_range(view.size(), newoffset, 8)?;
        let pos = table.fd_seek(fd, offset, whence)?;
        mem_write_u64(view, newoffset, pos)
    })())
}

pub fn fd_close(table: &mut Table, fd: i32) -> i32 {
    abi(table.close(fd))
}

pub fn fd_sync(table: &mut Table, fd: i32) -> i32 {
    abi(table.fd_sync(fd))
}

pub fn fd_filestat_set_size(table: &mut Table, fd: i32, size: i64) -> i32 {
    abi(table.fd_filestat_set_size(fd, size))
}

pub fn fd_fdstat_get(table: &mut Table, view: &impl GuestMemory, fd: i32, buf: i32) -> i32 {
    abi((|| {
        view.validate()?;
        let buf = guest_u32(buf)?;
        check_range(view.size(), buf, 24)?;
        let (filetype, fdflags, rights_base, rights_inheriting) = table.fdstat(fd)?;
        let mut bytes = [0u8; 24];
        bytes[0] = filetype;
        bytes[2..4].copy_from_slice(&fdflags.to_le_bytes());
        bytes[8..16].copy_from_slice(&rights_base.to_le_bytes());
        bytes[16..24].copy_from_slice(&rights_inheriting.to_le_bytes());
        mem_write(view, buf, &bytes)
    })())
}

pub fn fd_prestat_get(table: &mut Table, view: &impl GuestMemory, fd: i32, buf: i32) -> i32 {
    abi((|| {
        view.validate()?;
        let buf = guest_u32(buf)?;
        check_range(view.size(), buf, 8)?;
        let name = table.prestat(fd)?;
        let name_len = u32::try_from(name.len()).map_err(|_| NAMETOOLONG)?;
        let mut bytes = [0u8; 8];
        bytes[4..8].copy_from_slice(&name_len.to_le_bytes());
        mem_write(view, buf, &bytes)
    })())
}

pub fn fd_prestat_dir_name(
    table: &mut Table,
    view: &impl GuestMemory,
    fd: i32,
    path: i32,
    path_len: i32,
) -> i32 {
    abi((|| {
        let path_len = u32::try_from(path_len).map_err(|_| INVAL)?;
        view.validate()?;
        let path = guest_u32(path)?;
        check_range(view.size(), path, path_len)?;
        let name = table.prestat(fd)?;
        if name.len() > path_len as usize {
            return Err(NAMETOOLONG);
        }
        mem_write(view, path, name)
    })())
}

#[allow(clippy::too_many_arguments)]
pub fn path_open(
    table: &mut Table,
    view: &impl GuestMemory,
    dirfd: i32,
    dirflags: i32,
    path: i32,
    path_len: i32,
    oflags: i32,
    fs_rights_base: i64,
    fs_rights_inheriting: i64,
    fdflags: i32,
    fd: i32,
) -> i32 {
    abi((|| {
        let path_len = match u32::try_from(path_len) {
            Ok(n) if n > MAX_PATH => return Err(NAMETOOLONG),
            Ok(n) => n,
            Err(_) => return Err(INVAL),
        };
        let oflags = oflags as u16;
        let fdflags = fdflags as u16;
        view.validate()?;
        let path_ptr = guest_u32(path)?;
        let fd_ptr = guest_u32(fd)?;
        check_range(view.size(), path_ptr, path_len)?;
        check_range(view.size(), fd_ptr, 4)?;
        let mut bytes = vec![0u8; path_len as usize];
        if path_len != 0 {
            mem_read(view, path_ptr, &mut bytes)?;
        }
        let opened = table.path_open(
            dirfd,
            dirflags,
            &bytes,
            oflags,
            fs_rights_base as u64,
            fs_rights_inheriting as u64,
            fdflags,
        )?;
        mem_write_u32(view, fd_ptr, opened as u32)
    })())
}
