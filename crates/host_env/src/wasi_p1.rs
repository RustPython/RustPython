//! Subset of WASI preview1 used by the `wasm32-unknown-unknown` `host_io` guest.
//!
//! The guest imports `wasi_snapshot_preview1` functions and opens paths only
//! through preopened directories. The host decides which rights each open is
//! granted; this guest does not reject open flags on its own.

use std::io;
use std::sync::OnceLock;

use crate::errno::errors::{ENOENT, EOVERFLOW};
use crate::os::{
    O_ACCMODE, O_APPEND, O_CREAT, O_DIRECTORY, O_EXCL, O_NOFOLLOW, O_RDONLY, O_TRUNC, O_WRONLY,
};

const ERRNO_SUCCESS: i32 = 0;
const ERRNO_BADF: i32 = 8;

const FILETYPE_CHARACTER_DEVICE: u8 = 2;
pub(crate) const FILETYPE_DIRECTORY: u8 = 3;

const PRESTAT_DIR: u8 = 0;

const LOOKUP_SYMLINK_FOLLOW: i32 = 1;

const OFLAGS_CREAT: i32 = 1;
const OFLAGS_DIRECTORY: i32 = 2;
const OFLAGS_EXCL: i32 = 4;
const OFLAGS_TRUNC: i32 = 8;

const FDFLAGS_APPEND: i32 = 1;

const RIGHTS_FD_DATASYNC: u64 = 1 << 0;
const RIGHTS_FD_READ: u64 = 1 << 1;
const RIGHTS_FD_SEEK: u64 = 1 << 2;
const RIGHTS_FD_FDSTAT_SET_FLAGS: u64 = 1 << 3;
const RIGHTS_FD_SYNC: u64 = 1 << 4;
const RIGHTS_FD_TELL: u64 = 1 << 5;
const RIGHTS_FD_WRITE: u64 = 1 << 6;
const RIGHTS_FD_ADVISE: u64 = 1 << 7;
const RIGHTS_FD_ALLOCATE: u64 = 1 << 8;
const RIGHTS_FD_READDIR: u64 = 1 << 14;
const RIGHTS_FD_FILESTAT_GET: u64 = 1 << 21;
const RIGHTS_FD_FILESTAT_SET_SIZE: u64 = 1 << 22;
const RIGHTS_FD_FILESTAT_SET_TIMES: u64 = 1 << 23;
const RIGHTS_POLL_FD_READWRITE: u64 = 1 << 27;

const RIGHTS_ALL: u64 = (1 << 30) - 1;

const RIGHTS_BASE_ALWAYS: u64 = RIGHTS_FD_SEEK
    | RIGHTS_FD_TELL
    | RIGHTS_FD_SYNC
    | RIGHTS_FD_DATASYNC
    | RIGHTS_FD_FDSTAT_SET_FLAGS
    | RIGHTS_FD_ADVISE
    | RIGHTS_FD_FILESTAT_GET
    | RIGHTS_FD_FILESTAT_SET_TIMES
    | RIGHTS_FD_READDIR
    | RIGHTS_POLL_FD_READWRITE;

#[repr(C)]
struct IoVec {
    buf: u32,
    buf_len: u32,
}

const _: () = assert!(core::mem::size_of::<IoVec>() == 8);
const _: () = assert!(core::mem::align_of::<IoVec>() == 4);

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct FdStat {
    pub fs_filetype: u8,
    _pad0: u8,
    _fs_flags: u16,
    _pad1: u32,
    pub fs_rights_base: u64,
    _fs_rights_inheriting: u64,
}

const _: () = assert!(core::mem::size_of::<FdStat>() == 24);
const _: () = assert!(core::mem::align_of::<FdStat>() == 8);

#[repr(C)]
#[derive(Default)]
struct Prestat {
    tag: u8,
    _pad: [u8; 3],
    pr_name_len: u32,
}

const _: () = assert!(core::mem::size_of::<Prestat>() == 8);
const _: () = assert!(core::mem::align_of::<Prestat>() == 4);

#[link(wasm_import_module = "wasi_snapshot_preview1")]
unsafe extern "C" {
    fn fd_read(fd: i32, iovs: i32, iovs_len: i32, nread: i32) -> i32;
    fn fd_write(fd: i32, iovs: i32, iovs_len: i32, nwritten: i32) -> i32;
    fn fd_seek(fd: i32, offset: i64, whence: i32, newoffset: i32) -> i32;
    fn fd_close(fd: i32) -> i32;
    fn fd_sync(fd: i32) -> i32;
    fn fd_filestat_set_size(fd: i32, size: i64) -> i32;
    fn fd_fdstat_get(fd: i32, buf: i32) -> i32;
    fn fd_prestat_get(fd: i32, buf: i32) -> i32;
    fn fd_prestat_dir_name(fd: i32, path: i32, path_len: i32) -> i32;
    fn path_open(
        dirfd: i32,
        dirflags: i32,
        path: i32,
        path_len: i32,
        oflags: i32,
        fs_rights_base: i64,
        fs_rights_inheriting: i64,
        fdflags: i32,
        fd: i32,
    ) -> i32;
}

fn guest_ptr<T: ?Sized>(ptr: *const T) -> i32 {
    ptr as *const u8 as usize as i32
}

fn check(errno: i32) -> io::Result<()> {
    if errno == ERRNO_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(errno))
    }
}

fn checked_count(count: u32, len: usize) -> io::Result<usize> {
    let n = count as usize;
    if n <= len {
        Ok(n)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "WASI I/O count exceeds buffer length",
        ))
    }
}

fn io_once(fd: i32, buf: *const u8, len: usize, write: bool) -> io::Result<usize> {
    let iov = IoVec {
        buf: buf as u32,
        buf_len: len as u32,
    };
    let mut n: u32 = 0;
    let import = if write { fd_write } else { fd_read };
    // SAFETY: `iov` describes `buf`/`len` in this guest's linear memory, and `n`
    // is a valid out-pointer for the duration of the import.
    let errno = unsafe {
        import(
            fd,
            guest_ptr(&iov),
            1,
            guest_ptr(core::ptr::addr_of_mut!(n)),
        )
    };
    check(errno)?;
    checked_count(n, len)
}

pub(crate) fn read(fd: i32, buf: &mut [u8]) -> io::Result<usize> {
    io_once(fd, buf.as_mut_ptr(), buf.len(), false)
}

pub(crate) fn write(fd: i32, buf: &[u8]) -> io::Result<usize> {
    io_once(fd, buf.as_ptr(), buf.len(), true)
}

pub(crate) fn close(fd: i32) -> io::Result<()> {
    // SAFETY: `fd` is a host descriptor; close consumes it even when the import fails.
    check(unsafe { fd_close(fd) })
}

pub(crate) fn sync(fd: i32) -> io::Result<()> {
    // SAFETY: `fd` is a borrowed host descriptor for the duration of the import.
    check(unsafe { fd_sync(fd) })
}

pub(crate) fn set_size(fd: i32, size: i64) -> io::Result<()> {
    // SAFETY: `fd` is a borrowed host descriptor for the duration of the import.
    check(unsafe { fd_filestat_set_size(fd, size) })
}

pub(crate) fn seek(fd: i32, offset: i64, whence: i32) -> io::Result<i64> {
    let mut newoffset: u64 = 0;
    // SAFETY: `newoffset` is a valid out-pointer in this guest's linear memory.
    check(unsafe {
        fd_seek(
            fd,
            offset,
            whence,
            guest_ptr(core::ptr::addr_of_mut!(newoffset)),
        )
    })?;
    i64::try_from(newoffset).map_err(|_| io::Error::from_raw_os_error(EOVERFLOW))
}

pub(crate) fn fdstat(fd: i32) -> io::Result<FdStat> {
    let mut buf = FdStat::default();
    // SAFETY: `buf` is a valid fdstat out-buffer in this guest's linear memory.
    check(unsafe { fd_fdstat_get(fd, guest_ptr(core::ptr::addr_of_mut!(buf))) })?;
    Ok(buf)
}

pub(crate) fn is_tty(fd: i32) -> bool {
    let Ok(stat) = fdstat(fd) else {
        return false;
    };
    stat.fs_filetype == FILETYPE_CHARACTER_DEVICE
        && stat.fs_rights_base & (RIGHTS_FD_SEEK | RIGHTS_FD_TELL) == 0
}

fn normalize_preopen_name(name: &[u8]) -> Vec<u8> {
    let mut end = name.len();
    while end > 0 && name[end - 1] == b'/' {
        end -= 1;
    }
    let name = &name[..end];
    if name.is_empty() || name == b"." {
        b"/".to_vec()
    } else {
        name.to_vec()
    }
}

fn make_absolute(path: &[u8]) -> Vec<u8> {
    if path.starts_with(b"/") {
        path.to_vec()
    } else {
        let mut abs = Vec::with_capacity(path.len() + 1);
        abs.push(b'/');
        abs.extend_from_slice(path);
        abs
    }
}

fn is_component_prefix(prefix: &[u8], path: &[u8]) -> bool {
    if prefix == b"/" {
        return path.starts_with(b"/");
    }
    path.starts_with(prefix)
        && (path.len() == prefix.len() || path.get(prefix.len()) == Some(&b'/'))
}

fn relative_from_prefix(path: &[u8], prefix: &[u8]) -> Vec<u8> {
    let rest = if prefix == b"/" {
        &path[1..]
    } else if path.len() == prefix.len() {
        &[]
    } else {
        &path[prefix.len() + 1..]
    };
    if rest.is_empty() {
        b".".to_vec()
    } else {
        rest.to_vec()
    }
}

fn enumerate_preopens() -> Vec<(i32, Vec<u8>)> {
    let mut preopens = Vec::new();
    let mut fd = 3i32;
    loop {
        let mut prestat = Prestat::default();
        // SAFETY: `prestat` is a valid out-buffer in this guest's linear memory.
        let errno = unsafe { fd_prestat_get(fd, guest_ptr(core::ptr::addr_of_mut!(prestat))) };
        if errno == ERRNO_BADF {
            break;
        }
        if errno == ERRNO_SUCCESS && prestat.tag == PRESTAT_DIR {
            let mut name = vec![0u8; prestat.pr_name_len as usize];
            // SAFETY: `name` is `pr_name_len` bytes of guest linear memory.
            let errno =
                unsafe { fd_prestat_dir_name(fd, guest_ptr(name.as_mut_ptr()), name.len() as i32) };
            if errno == ERRNO_SUCCESS {
                preopens.push((fd, normalize_preopen_name(&name)));
            }
        }
        let Some(next) = fd.checked_add(1) else {
            break;
        };
        fd = next;
    }
    preopens
}

fn preopens() -> &'static [(i32, Vec<u8>)] {
    static PREOPENS: OnceLock<Vec<(i32, Vec<u8>)>> = OnceLock::new();
    PREOPENS.get_or_init(enumerate_preopens)
}

fn resolve_preopen(path: &[u8]) -> io::Result<(i32, Vec<u8>)> {
    let abs = make_absolute(path);
    let mut best: Option<(usize, i32)> = None;
    for &(fd, ref name) in preopens() {
        if is_component_prefix(name, &abs) && best.is_none_or(|(len, _)| name.len() > len) {
            best = Some((name.len(), fd));
        }
    }
    let Some((prefix_len, fd)) = best else {
        return Err(io::Error::from_raw_os_error(ENOENT));
    };
    let prefix = &abs[..prefix_len];
    Ok((fd, relative_from_prefix(&abs, prefix)))
}

fn oflags_from_open_flags(flags: i32) -> i32 {
    let mut oflags = 0;
    if flags & O_CREAT != 0 {
        oflags |= OFLAGS_CREAT;
    }
    if flags & O_DIRECTORY != 0 {
        oflags |= OFLAGS_DIRECTORY;
    }
    if flags & O_EXCL != 0 {
        oflags |= OFLAGS_EXCL;
    }
    if flags & O_TRUNC != 0 {
        oflags |= OFLAGS_TRUNC;
    }
    oflags
}

fn rights_base_from_open_flags(flags: i32) -> u64 {
    let accmode = flags & O_ACCMODE;
    let mut rights = RIGHTS_BASE_ALWAYS;
    if accmode != O_WRONLY {
        rights |= RIGHTS_FD_READ;
    }
    if accmode != O_RDONLY {
        rights |= RIGHTS_FD_WRITE | RIGHTS_FD_ALLOCATE | RIGHTS_FD_FILESTAT_SET_SIZE;
    }
    rights
}

pub(crate) fn path_open_preopen(path: &[u8], flags: i32) -> io::Result<i32> {
    let (dirfd, rel) = resolve_preopen(path)?;
    let dirflags = if flags & O_NOFOLLOW == 0 {
        LOOKUP_SYMLINK_FOLLOW
    } else {
        0
    };
    let oflags = oflags_from_open_flags(flags);
    let fdflags = if flags & O_APPEND != 0 {
        FDFLAGS_APPEND
    } else {
        0
    };
    let rights_base = rights_base_from_open_flags(flags);
    let mut opened: i32 = 0;
    // SAFETY: `rel` and `opened` are valid guest buffers for the duration of the import.
    check(unsafe {
        path_open(
            dirfd,
            dirflags,
            guest_ptr(rel.as_ptr()),
            rel.len() as i32,
            oflags,
            rights_base as i64,
            RIGHTS_ALL as i64,
            fdflags,
            guest_ptr(core::ptr::addr_of_mut!(opened)),
        )
    })?;
    Ok(opened)
}
