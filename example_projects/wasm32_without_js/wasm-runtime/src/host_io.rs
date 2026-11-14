//! Opt-in descriptor ABI. Guest integers index this table, never native descriptors.
use rustpython_host_env::{crt_fd, io as host_io, os::ErrorExt};
#[cfg(unix)]
use std::ffi::CString;
use std::{io, path::Path};
use wasmer::{FunctionEnvMut, RuntimeError};

use super::Context;

// The guest uses BSD errno numbers, even when the host runs Linux or Windows.
fn errno(error: io::Error) -> i64 {
    use rustpython_host_env::errno::errors as native;
    let posix = error.posix_errno();
    // Capability denials are Rust errors, not OS errors. Windows maps a
    // missing raw OS error through winerror_to_errno(0), so checking only
    // posix == 0 would misreport a denied path as EINVAL there.
    if error.raw_os_error().is_none() {
        match error.kind() {
            io::ErrorKind::PermissionDenied => return -13,
            io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => return -22,
            io::ErrorKind::Unsupported => return -45,
            _ => {}
        }
    }
    let mapped = match posix {
        native::EPERM => 1,
        native::ENOENT => 2,
        native::EINTR => 4,
        native::EBADF => 9,
        native::EACCES => 13,
        native::EEXIST => 17,
        native::ENOTDIR => 20,
        native::EISDIR => 21,
        native::EINVAL => 22,
        native::EMFILE => 24,
        native::EFBIG => 27,
        native::ENOSPC => 28,
        native::ESPIPE => 29,
        native::EROFS => 30,
        native::EPIPE => 32,
        native::EAGAIN => 35,
        native::ENAMETOOLONG => 63,
        native::EOVERFLOW => 84,
        _ => 5, // Unrepresented native failures become EIO, not an unrelated errno.
    };
    -mapped
}

#[derive(Default)]
pub struct Descriptors {
    entries: Vec<Option<crt_fd::Owned>>,
}

impl Descriptors {
    fn insert_readonly(&mut self, file: std::fs::File) -> io::Result<i32> {
        let index = self
            .entries
            .iter()
            .enumerate()
            .skip(3)
            .find_map(|(index, entry)| entry.is_none().then_some(index))
            .unwrap_or(self.entries.len().max(3));
        let raw = i32::try_from(index).map_err(|_| io::Error::other("descriptor table full"))?;
        #[cfg(unix)]
        let owned = {
            let file: std::os::fd::OwnedFd = file.into();
            crt_fd::Owned::from(file)
        };
        #[cfg(windows)]
        let owned = {
            use std::os::windows::io::{AsRawHandle, IntoRawHandle};
            let fd = rustpython_host_env::msvcrt::open_osfhandle(
                file.as_raw_handle() as isize,
                rustpython_host_env::os::O_RDONLY | rustpython_host_env::os::O_BINARY,
            )?;
            let _ = file.into_raw_handle();
            unsafe { crt_fd::Owned::from_raw(fd) }
        };
        if index >= self.entries.len() {
            self.entries.resize_with(index + 1, || None);
        }
        self.entries[index] = Some(owned);
        Ok(raw)
    }

    pub fn with_file(path: &Path) -> io::Result<Self> {
        // No ambient stdin/stdout/stderr. The sole explicitly lent file is guest FD 3.
        #[cfg(unix)]
        let file = {
            use std::os::unix::ffi::OsStrExt;
            let path = CString::new(path.as_os_str().as_bytes())?;
            host_io::open_path(&path, rustpython_host_env::os::O_RDWR, 0)?
        };
        #[cfg(windows)]
        let file = {
            use std::os::windows::ffi::OsStrExt;
            let path = widestring::WideCString::from_vec(
                path.as_os_str().encode_wide().collect::<Vec<_>>(),
            )
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
            let file = host_io::open_path(&path, rustpython_host_env::os::O_RDWR, 0)?;
            rustpython_host_env::msvcrt::setmode_binary(file.borrow());
            file
        };
        Ok(Self {
            entries: vec![None, None, None, Some(file)],
        })
    }

    fn get(&self, fd: i32) -> Result<crt_fd::Borrowed<'_>, i64> {
        usize::try_from(fd)
            .ok()
            .and_then(|fd| self.entries.get(fd))
            .and_then(Option::as_ref)
            .map(crt_fd::Owned::borrow)
            .ok_or(-9)
    }

    fn close(&mut self, fd: i32) -> i64 {
        let Some(file) = usize::try_from(fd)
            .ok()
            .and_then(|fd| self.entries.get_mut(fd))
            .and_then(Option::take)
        else {
            return -9;
        };
        // Remove before closing: neither an error nor table destruction retries close.
        crt_fd::close(file).map_or_else(errno, |()| 0)
    }
}

// Bound scratch allocation and permit short I/O. Validate the *whole* guest range
// first, before consuming pipe data or changing a file offset.
const MAX_TRANSFER: usize = 64 * 1024;
fn check_range(size: u64, ptr: u32, len: u32) -> Result<(), RuntimeError> {
    if u64::from(ptr) + u64::from(len) > size {
        Err(RuntimeError::new("host I/O memory range is out of bounds"))
    } else {
        Ok(())
    }
}

pub fn fd_read(
    mut ctx: FunctionEnvMut<Context>,
    fd: i32,
    ptr: u32,
    len: u32,
) -> Result<i64, RuntimeError> {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context.memory()?.view(&store);
    check_range(memory.data_size(), ptr, len)?;
    let file = match context.descriptors.get(fd) {
        Ok(fd) => fd,
        Err(e) => return Ok(e),
    };
    let mut bytes = vec![0; (len as usize).min(MAX_TRANSFER)];
    match crt_fd::read(file, &mut bytes) {
        Ok(n) => {
            memory
                .write(u64::from(ptr), &bytes[..n])
                .map_err(|e| RuntimeError::new(e.to_string()))?;
            Ok(n as i64)
        }
        Err(e) => Ok(errno(e)),
    }
}

pub fn fd_write(
    mut ctx: FunctionEnvMut<Context>,
    fd: i32,
    ptr: u32,
    len: u32,
) -> Result<i64, RuntimeError> {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context.memory()?.view(&store);
    check_range(memory.data_size(), ptr, len)?;
    let file = match context.descriptors.get(fd) {
        Ok(fd) => fd,
        Err(e) => return Ok(e),
    };
    let mut bytes = vec![0; (len as usize).min(MAX_TRANSFER)];
    memory
        .read(u64::from(ptr), &mut bytes)
        .map_err(|e| RuntimeError::new(e.to_string()))?;
    Ok(crt_fd::write(file, &bytes).map_or_else(errno, |n| n as i64))
}

pub fn fd_close(mut ctx: FunctionEnvMut<Context>, fd: i32) -> i64 {
    ctx.data_mut().descriptors.close(fd)
}
pub fn fd_seek(ctx: FunctionEnvMut<Context>, fd: i32, offset: i64, whence: i32) -> i64 {
    if !(0..=2).contains(&whence) {
        return -22;
    }
    let file = match ctx.data().descriptors.get(fd) {
        Ok(fd) => fd,
        Err(e) => return e,
    };
    host_io::seek(file, offset, whence).unwrap_or_else(errno)
}
pub fn fd_truncate(ctx: FunctionEnvMut<Context>, fd: i32, len: i64) -> i64 {
    if len < 0 {
        return -22;
    }
    let file = match ctx.data().descriptors.get(fd) {
        Ok(fd) => fd,
        Err(e) => return e,
    };
    crt_fd::ftruncate(file, len).map_or_else(errno, |()| 0)
}
pub fn fd_sync(ctx: FunctionEnvMut<Context>, fd: i32) -> i64 {
    let file = match ctx.data().descriptors.get(fd) {
        Ok(fd) => fd,
        Err(e) => return e,
    };
    crt_fd::fsync(file).map_or_else(errno, |()| 0)
}
pub fn fd_info(ctx: FunctionEnvMut<Context>, fd: i32) -> i64 {
    let file = match ctx.data().descriptors.get(fd) {
        Ok(fd) => fd,
        Err(e) => return e,
    };
    if let Err(e) = host_io::inspect_file_target(file) {
        return errno(e);
    }
    i64::from(host_io::reads_without_waiting(file)) * 2
        + i64::from(host_io::isatty(file.as_raw())) * 4
}

pub fn path_open(
    mut ctx: FunctionEnvMut<Context>,
    ptr: u32,
    len: u32,
    flags: i32,
) -> Result<i64, RuntimeError> {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context.memory()?.view(&store);
    check_range(memory.data_size(), ptr, len)?;
    if flags != 0 {
        return Ok(-30);
    }
    if len > 65536 {
        return Ok(-63);
    }
    let Some(root) = context.root.as_ref() else {
        return Ok(-13);
    };
    let mut bytes = vec![0; len as usize];
    memory
        .read(u64::from(ptr), &mut bytes)
        .map_err(|e| RuntimeError::new(e.to_string()))?;
    if bytes.contains(&0) {
        return Ok(-22);
    }
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStrExt;
        Path::new(std::ffi::OsStr::from_bytes(&bytes))
    };
    #[cfg(not(unix))]
    let path = match std::str::from_utf8(&bytes) {
        Ok(path) => Path::new(path),
        Err(_) => return Ok(-22),
    };
    let file = match root.open_read(path) {
        Ok(file) => file,
        Err(error) => return Ok(errno(error)),
    };
    Ok(context
        .descriptors
        .insert_readonly(file)
        .map_or_else(errno, i64::from))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_errors_keep_their_error_class() {
        assert_eq!(
            errno(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "root escape"
            )),
            -13
        );
        assert_eq!(
            errno(io::Error::new(io::ErrorKind::InvalidInput, "encoding")),
            -22
        );
        assert_eq!(
            errno(io::Error::new(io::ErrorKind::Unsupported, "unsupported")),
            -45
        );
    }

    #[test]
    fn guest_range_is_checked_without_allocation() {
        assert!(check_range(65536, 65536, 0).is_ok());
        assert!(check_range(65536, 65536, 1).is_err());
        assert!(check_range(65536, u32::MAX, u32::MAX).is_err());
    }
    #[test]
    fn invalid_guest_descriptors_do_not_address_host_files() {
        let mut descriptors = Descriptors::default();
        assert_eq!(descriptors.get(0).unwrap_err(), -9);
        assert_eq!(descriptors.get(-1).unwrap_err(), -9);
        assert_eq!(descriptors.close(3), -9);
    }
    #[test]
    #[cfg(unix)]
    fn native_would_block_is_translated() {
        assert_eq!(
            errno(io::Error::from_raw_os_error(
                rustpython_host_env::errno::errors::EAGAIN
            )),
            -35
        );
    }
    #[test]
    #[cfg(unix)]
    fn wasm_nonblocking_stream_and_invalid_memory_preserves_input() {
        use std::io::Write;
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;
        use wasmer::{Function, FunctionEnv, Instance, Module, Store, imports};
        let (reader, mut writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let owned: OwnedFd = reader.into();
        let mut store = Store::default();
        let env = FunctionEnv::new(
            &mut store,
            Context {
                memory: None,
                kv: Default::default(),
                root: None,
                descriptors: Descriptors {
                    entries: vec![None, None, None, Some(owned.into())],
                },
            },
        );
        let module = Module::new(
            &store,
            br#"(module
            (import "io" "read" (func $read (param i32 i32 i32) (result i64)))
            (import "io" "seek" (func $seek (param i32 i64 i32) (result i64)))
            (import "io" "close" (func $close (param i32) (result i64)))
            (memory (export "memory") 1)
            (export "read" (func $read))
            (export "seek" (func $seek))
            (export "close" (func $close)))"#,
        )
        .unwrap();
        let imports = imports! { "io" => {
            "read" => Function::new_typed_with_env(&mut store, &env, fd_read),
            "seek" => Function::new_typed_with_env(&mut store, &env, fd_seek),
            "close" => Function::new_typed_with_env(&mut store, &env, fd_close),
        }};
        let instance = Instance::new(&mut store, &module, &imports).unwrap();
        let memory = instance.exports.get_memory("memory").unwrap().clone();
        env.as_mut(&mut store).memory = Some(memory.clone());
        let read = instance
            .exports
            .get_typed_function::<(i32, u32, u32), i64>(&store, "read")
            .unwrap();
        let seek = instance
            .exports
            .get_typed_function::<(i32, i64, i32), i64>(&store, "seek")
            .unwrap();
        let close = instance
            .exports
            .get_typed_function::<i32, i64>(&store, "close")
            .unwrap();
        assert_eq!(read.call(&mut store, 3, 0, 3).unwrap(), -35);
        assert_eq!(seek.call(&mut store, 3, 0, 0).unwrap(), -29);
        writer.write_all(b"abc").unwrap();
        assert!(read.call(&mut store, 3, 65535, 3).is_err());
        assert_eq!(read.call(&mut store, 3, 0, 3).unwrap(), 3);
        let mut bytes = [0; 3];
        memory.view(&store).read(0, &mut bytes).unwrap();
        assert_eq!(&bytes, b"abc");
        drop(writer);
        assert_eq!(read.call(&mut store, 3, 0, 3).unwrap(), 0);
        assert_eq!(close.call(&mut store, 3).unwrap(), 0);
        assert_eq!(close.call(&mut store, 3).unwrap(), -9);
        assert_eq!(read.call(&mut store, 3, 0, 3).unwrap(), -9);
    }
}
