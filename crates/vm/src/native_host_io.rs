// cspell:ignore ACCES BADF DEADLK ILSEQ MFILE NOSPC SPIPE TXTBSY FDFLAGS fdflags fdstat
//! Interpreter-owned native capability I/O, without ambient descriptor authority.
use crate::common::{lock::PyMutex, rc::PyRc};
use crate::host_env::{errno::errors as errno, os::ErrorExt};
use core::ffi::CStr;
use rustpython_wasi_host::{self as wasi, Handle, Table};
use std::io;

#[derive(Clone, Default)]
pub(crate) struct HostIo(PyRc<PyMutex<Table>>);
impl From<Table> for HostIo {
    fn from(table: Table) -> Self {
        Self(PyRc::new(PyMutex::new(table)))
    }
}
impl HostIo {
    pub(crate) fn pin(&self, fd: i32) -> io::Result<Handle> {
        self.0.lock().pin(fd).map_err(native_error)
    }
    #[cfg(feature = "stdio")]
    pub(crate) fn contains(&self, fd: i32) -> bool {
        self.pin(fd).is_ok()
    }
    pub(crate) fn close(&self, fd: i32) -> io::Result<()> {
        let detached = self.0.lock().take(fd).map_err(native_error)?;
        drop(detached);
        Ok(())
    }
    pub(crate) fn write_stderr(&self, args: core::fmt::Arguments<'_>) {
        let Ok(handle) = self.pin(2) else { return };
        let message = args.to_string();
        let mut bytes = message.as_bytes();
        while !bytes.is_empty() {
            match handle.fd_write(&[bytes]) {
                Ok(0) | Err(_) => break,
                Ok(n) => bytes = &bytes[n..],
            }
        }
    }
    pub(crate) fn open_path(&self, path: &CStr, flags: i32) -> io::Result<i32> {
        use crate::host_env::os::{O_APPEND, O_CREAT, O_EXCL, O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY};
        let (directory, relative) = self
            .0
            .lock()
            .resolve_preopen(path.to_bytes())
            .map_err(native_error)?;
        let access = flags & (O_WRONLY | O_RDWR);
        let mut rights = wasi::RIGHTS_FD_SEEK | wasi::RIGHTS_FD_TELL | wasi::RIGHTS_FD_SYNC;
        if access == O_RDONLY || access == O_RDWR {
            rights |= wasi::RIGHTS_FD_READ;
        }
        if access == O_WRONLY || access == O_RDWR {
            rights |= wasi::RIGHTS_FD_WRITE | wasi::RIGHTS_FD_FILESTAT_SET_SIZE;
        }
        let mut oflags = 0;
        if flags & O_CREAT != 0 {
            oflags |= wasi::OFLAGS_CREAT;
        }
        if flags & O_EXCL != 0 {
            oflags |= wasi::OFLAGS_EXCL;
        }
        if flags & O_TRUNC != 0 {
            oflags |= wasi::OFLAGS_TRUNC;
        }
        let fdflags = if flags & O_APPEND != 0 {
            wasi::FDFLAGS_APPEND
        } else {
            0
        };
        let handle = directory
            .open_child(&relative, oflags, rights, 0, fdflags)
            .map_err(native_error)?;
        let inserted = self.0.lock().try_insert(handle);
        inserted.map_err(|(error, rejected)| {
            drop(rejected);
            native_error(error)
        })
    }
}
pub(crate) struct Descriptor(PyMutex<(Option<(HostIo, i32)>, bool)>);
impl Default for Descriptor {
    fn default() -> Self {
        Self(PyMutex::new((None, true)))
    }
}
impl core::fmt::Debug for Descriptor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Descriptor").field(&self.load()).finish()
    }
}
impl Descriptor {
    pub(crate) fn load(&self) -> i32 {
        self.0.lock().0.as_ref().map_or(-1, |(_, fd)| *fd)
    }
    pub(crate) fn bind(&self, owner: HostIo, fd: i32, closefd: bool) -> io::Result<()> {
        let (old, owned) = {
            let mut state = self.0.lock();
            let same = state
                .0
                .as_ref()
                .is_some_and(|(prior, old_fd)| *old_fd == fd && PyRc::ptr_eq(&prior.0, &owner.0));
            let old = state.0.replace((owner, fd));
            let owned = core::mem::replace(&mut state.1, closefd);
            (old, owned && !same)
        };
        if owned && let Some((owner, fd)) = old {
            match owner.close(fd) {
                Err(e) if e.posix_errno() == errno::EBADF => {}
                result => result?,
            }
        }
        Ok(())
    }
    pub(crate) fn closefd(&self) -> bool {
        self.0.lock().1
    }
    pub(crate) fn forget(&self) {
        let old = self.0.lock().0.take();
        drop(old);
    }
    pub(crate) fn pin(&self) -> io::Result<Handle> {
        let binding = self.0.lock().0.clone();
        let (owner, fd) = binding.ok_or_else(|| native_error(wasi::BADF))?;
        owner.pin(fd)
    }
    pub(crate) fn close(&self) -> io::Result<()> {
        let (old, owned) = {
            let mut s = self.0.lock();
            (s.0.take(), s.1)
        };
        if owned && let Some((owner, fd)) = old {
            owner.close(fd)?;
        }
        Ok(())
    }
}
pub(crate) fn native_error(code: u16) -> io::Error {
    let code = match code {
        wasi::TOOBIG => errno::E2BIG,
        wasi::ACCES => errno::EACCES,
        wasi::ADDRINUSE => errno::EADDRINUSE,
        wasi::ADDRNOTAVAIL => errno::EADDRNOTAVAIL,
        wasi::AGAIN => errno::EAGAIN,
        wasi::BADF => errno::EBADF,
        wasi::BUSY => errno::EBUSY,
        wasi::CONNABORTED => errno::ECONNABORTED,
        wasi::CONNREFUSED => errno::ECONNREFUSED,
        wasi::CONNRESET => errno::ECONNRESET,
        wasi::DEADLK => errno::EDEADLK,
        wasi::DQUOT => errno::EDQUOT,
        wasi::EXIST => errno::EEXIST,
        wasi::FAULT => errno::EFAULT,
        wasi::FBIG => errno::EFBIG,
        wasi::ILSEQ => errno::EILSEQ,
        wasi::INTR => errno::EINTR,
        wasi::INVAL => errno::EINVAL,
        wasi::IO => errno::EIO,
        wasi::ISDIR => errno::EISDIR,
        wasi::MFILE => errno::EMFILE,
        wasi::MLINK => errno::EMLINK,
        wasi::NAMETOOLONG => errno::ENAMETOOLONG,
        wasi::NOENT => errno::ENOENT,
        wasi::NOMEM => errno::ENOMEM,
        wasi::NOSPC => errno::ENOSPC,
        wasi::NOTSUP => errno::ENOTSUP,
        wasi::NOTDIR => errno::ENOTDIR,
        wasi::NOTEMPTY => errno::ENOTEMPTY,
        wasi::NOTCONN => errno::ENOTCONN,
        wasi::OVERFLOW => errno::EOVERFLOW,
        wasi::PERM => errno::EPERM,
        wasi::PIPE => errno::EPIPE,
        wasi::ROFS => errno::EROFS,
        wasi::SPIPE => errno::ESPIPE,
        wasi::STALE => errno::ESTALE,
        wasi::TIMEDOUT => errno::ETIMEDOUT,
        wasi::TXTBSY => errno::ETXTBSY,
        wasi::XDEV => errno::EXDEV,
        wasi::NOTCAPABLE => errno::EACCES,
        _ => errno::EIO,
    };
    #[cfg(windows)]
    {
        crate::host_env::os::io_error_from_errno(code)
    }
    #[cfg(not(windows))]
    {
        io::Error::from_raw_os_error(code)
    }
}
pub(crate) mod file_ops {
    use super::*;
    pub(crate) use rustpython_host_env::io::{FileMode, parse_fileio_mode};
    pub(crate) fn inspect_file_target(
        fd: &Handle,
    ) -> io::Result<rustpython_host_env::io::FileTargetInfo> {
        if fd.fdstat().map_err(native_error)?.0 == wasi::FILETYPE_DIRECTORY {
            return Err(native_error(wasi::ISDIR));
        }
        Ok(rustpython_host_env::io::FileTargetInfo { blksize: None })
    }
    pub(crate) fn should_forget_fd_after_inspect_error(e: &io::Error, owned: bool) -> bool {
        e.posix_errno() == errno::EBADF || (!owned && e.posix_errno() == errno::EISDIR)
    }
    pub(crate) fn read_once(fd: &Handle, buf: &mut [u8]) -> io::Result<usize> {
        fd.fd_read(&mut [buf]).map_err(native_error)
    }
    pub(crate) fn write_once(fd: &Handle, buf: &[u8]) -> io::Result<usize> {
        fd.fd_write(&[buf]).map_err(native_error)
    }
    pub(crate) fn seek(fd: &Handle, offset: i64, how: i32) -> io::Result<i64> {
        fd.fd_seek(offset, how)
            .and_then(|v| i64::try_from(v).map_err(|_| wasi::OVERFLOW))
            .map_err(native_error)
    }
    pub(crate) fn tell(fd: &Handle) -> io::Result<i64> {
        seek(fd, 0, 1)
    }
    pub(crate) fn seek_to_end(fd: &Handle) -> io::Result<i64> {
        seek(fd, 0, 2)
    }
    pub(crate) fn is_seekable(fd: &Handle) -> bool {
        tell(fd).is_ok()
    }
    pub(crate) fn reads_without_waiting(fd: &Handle) -> bool {
        fd.fdstat()
            .is_ok_and(|s| s.0 == wasi::FILETYPE_REGULAR_FILE)
    }
    pub(crate) fn truncate(fd: &Handle, len: i64) -> io::Result<()> {
        fd.fd_filestat_set_size(len).map_err(native_error)
    }
    pub(crate) fn is_interrupted_error(e: &io::Error) -> bool {
        e.posix_errno() == errno::EINTR
    }
    pub(crate) fn is_would_block_error(e: &io::Error) -> bool {
        e.posix_errno() == errno::EAGAIN
    }
}
