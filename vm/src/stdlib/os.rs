use std::ffi;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use std::{env, fs};

use crossbeam_utils::atomic::AtomicCell;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use filepath::FilePath;
use num_bigint::BigInt;

use super::errno::errors;
use crate::builtins::bytes::{PyBytes, PyBytesRef};
use crate::builtins::dict::PyDictRef;
use crate::builtins::int::{self, PyIntRef};
use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::set::PySet;
use crate::builtins::tuple::{PyTuple, PyTupleRef};
use crate::byteslike::PyBytesLike;
use crate::common::lock::PyRwLock;
use crate::exceptions::{IntoPyException, PyBaseExceptionRef};
use crate::function::{ArgumentError, FromArgs, FuncArgs, IntoPyNativeFunc, OptionalArg};
use crate::pyobject::{
    BorrowValue, Either, IntoPyObject, ItemProtocol, PyObjectRef, PyRef, PyResult,
    PyStructSequence, PyValue, StaticType, TryFromObject, TypeProtocol,
};
use crate::slots::PyIter;
use crate::vm::VirtualMachine;

// this is basically what CPython has for Py_off_t; windows uses long long
// for offsets, other platforms just use off_t
#[cfg(not(windows))]
pub type Offset = libc::off_t;
#[cfg(windows)]
pub type Offset = libc::c_longlong;

#[derive(Debug, Copy, Clone)]
enum OutputMode {
    String,
    Bytes,
}

impl OutputMode {
    fn process_path(self, path: impl Into<PathBuf>, vm: &VirtualMachine) -> PyResult {
        fn inner(mode: OutputMode, path: PathBuf, vm: &VirtualMachine) -> PyResult {
            let path_as_string = |p: PathBuf| {
                p.into_os_string().into_string().map_err(|_| {
                    vm.new_unicode_decode_error(
                        "Can't convert OS path to valid UTF-8 string".into(),
                    )
                })
            };
            match mode {
                OutputMode::String => path_as_string(path).map(|s| vm.ctx.new_str(s)),
                OutputMode::Bytes => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::ffi::OsStringExt;
                        Ok(vm.ctx.new_bytes(path.into_os_string().into_vec()))
                    }
                    #[cfg(target_os = "wasi")]
                    {
                        use std::os::wasi::ffi::OsStringExt;
                        Ok(vm.ctx.new_bytes(path.into_os_string().into_vec()))
                    }
                    #[cfg(windows)]
                    {
                        path_as_string(path).map(|s| vm.ctx.new_bytes(s.into_bytes()))
                    }
                }
            }
        }
        inner(self, path.into(), vm)
    }
}

fn osstr_contains_nul(s: &ffi::OsStr) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        s.as_bytes().contains(&b'\0')
    }
    #[cfg(target_os = "wasi")]
    {
        use std::os::wasi::ffi::OsStrExt;
        s.as_bytes().contains(&b'\0')
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().any(|c| c == 0)
    }
}

pub struct PyPathLike {
    pub path: PathBuf,
    mode: OutputMode,
}

impl PyPathLike {
    pub fn new_str(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mode: OutputMode::String,
        }
    }
}

fn fs_metadata<P: AsRef<Path>>(path: P, follow_symlink: bool) -> io::Result<fs::Metadata> {
    if follow_symlink {
        fs::metadata(path.as_ref())
    } else {
        fs::symlink_metadata(path.as_ref())
    }
}

impl AsRef<Path> for PyPathLike {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl TryFromObject for PyPathLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let match1 = |obj: &PyObjectRef| {
            let pathlike = match_class!(match obj {
                ref l @ PyStr => PyPathLike {
                    path: l.borrow_value().into(),
                    mode: OutputMode::String,
                },
                ref i @ PyBytes => PyPathLike {
                    path: bytes_as_osstr(&i, vm)?.to_os_string().into(),
                    mode: OutputMode::Bytes,
                },
                _ => return Ok(None),
            });
            Ok(Some(pathlike))
        };
        if let Some(pathlike) = match1(&obj)? {
            return Ok(pathlike);
        }
        let method = vm.get_method_or_type_error(obj.clone(), "__fspath__", || {
            format!(
                "expected str, bytes or os.PathLike object, not '{}'",
                obj.class().name
            )
        })?;
        let result = vm.invoke(&method, ())?;
        match1(&result)?.ok_or_else(|| {
            vm.new_type_error(format!(
                "expected {}.__fspath__() to return str or bytes, not '{}'",
                obj.class().name,
                result.class().name,
            ))
        })
    }
}

fn path_from_fd(raw_fd: i64, vm: &VirtualMachine) -> PyResult<PathBuf> {
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "linux", target_os = "macos", windows))] {
            let file = rust_file(raw_fd);
            let res = file.path().map_err(|e| e.into_pyexception(vm));
            raw_file_number(file);  // Do not consume `raw_fd`
            res
        } else {
            Err(vm.new_not_implemented_error("dir_fd unavailable on this platform".to_owned()))
        }
    }
}

fn make_path<'a>(
    vm: &VirtualMachine,
    path: &'a PyPathLike,
    dir_fd: &DirFd,
) -> PyResult<std::borrow::Cow<'a, ffi::OsStr>> {
    let path = &path.path;
    if dir_fd.0.is_none() || path.is_absolute() {
        return Ok(path.as_os_str().into());
    }
    let dir_path = path_from_fd(dir_fd.0.unwrap().into(), vm)?;
    let p: PathBuf = vec![dir_path, path.to_path_buf()].iter().collect();
    Ok(p.into_os_string().into())
}

impl IntoPyException for io::Error {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        (&self).into_pyexception(vm)
    }
}
impl IntoPyException for &'_ io::Error {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        #[allow(unreachable_patterns)] // some errors are just aliases of each other
        let exc_type = match self.kind() {
            ErrorKind::NotFound => vm.ctx.exceptions.file_not_found_error.clone(),
            ErrorKind::PermissionDenied => vm.ctx.exceptions.permission_error.clone(),
            ErrorKind::AlreadyExists => vm.ctx.exceptions.file_exists_error.clone(),
            ErrorKind::WouldBlock => vm.ctx.exceptions.blocking_io_error.clone(),
            _ => match self.raw_os_error() {
                Some(errors::EAGAIN)
                | Some(errors::EALREADY)
                | Some(errors::EWOULDBLOCK)
                | Some(errors::EINPROGRESS) => vm.ctx.exceptions.blocking_io_error.clone(),
                Some(errors::ESRCH) => vm.ctx.exceptions.process_lookup_error.clone(),
                _ => vm.ctx.exceptions.os_error.clone(),
            },
        };
        let errno = self.raw_os_error().into_pyobject(vm);
        let msg = vm.ctx.new_str(self.to_string());
        vm.new_exception(exc_type, vec![errno, msg])
    }
}

#[cfg(unix)]
impl IntoPyException for nix::Error {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match self {
            nix::Error::InvalidPath => {
                let exc_type = vm.ctx.exceptions.file_not_found_error.clone();
                vm.new_exception_msg(exc_type, self.to_string())
            }
            nix::Error::InvalidUtf8 => {
                let exc_type = vm.ctx.exceptions.unicode_error.clone();
                vm.new_exception_msg(exc_type, self.to_string())
            }
            nix::Error::UnsupportedOperation => vm.new_runtime_error(self.to_string()),
            nix::Error::Sys(errno) => {
                let exc_type = posix::convert_nix_errno(vm, errno);
                vm.new_exception(
                    exc_type,
                    vec![
                        vm.ctx.new_int(errno as i32),
                        vm.ctx.new_str(self.to_string()),
                    ],
                )
            }
        }
    }
}

/// Convert the error stored in the `errno` variable into an Exception
#[inline]
pub fn errno_err(vm: &VirtualMachine) -> PyBaseExceptionRef {
    io::Error::last_os_error().into_pyexception(vm)
}

#[allow(dead_code)]
#[derive(FromArgs, Default)]
pub struct TargetIsDirectory {
    #[pyarg(any, default = "false")]
    target_is_directory: bool,
}

// TODO: we can have a const AVAIL: bool const generic argument once min_const_generics to mimic
// CPython's dir_fd_unavailable
#[derive(Default)]
pub struct DirFd(Option<i32>);

impl FromArgs for DirFd {
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let fd = match args.take_keyword("dir_fd") {
            Some(o) if vm.is_none(&o) => None,
            Some(o) => {
                let fd = vm.to_index_opt(o.clone()).unwrap_or_else(|| {
                    Err(vm.new_type_error(format!(
                        "argument should be integer or None, not {}",
                        o.class().name
                    )))
                })?;
                let fd = int::try_to_primitive(fd.borrow_value(), vm)?;
                Some(fd)
            }
            None => None,
        };
        Ok(Self(fd))
    }
}

#[derive(FromArgs)]
struct FollowSymlinks(#[pyarg(named, name = "follow_symlinks", default = "true")] bool);

#[cfg(unix)]
use posix::bytes_as_osstr;

#[cfg(not(unix))]
fn bytes_as_osstr<'a>(b: &'a [u8], vm: &VirtualMachine) -> PyResult<&'a ffi::OsStr> {
    std::str::from_utf8(b)
        .map(|s| s.as_ref())
        .map_err(|_| vm.new_value_error("Can't convert bytes to str for env function".to_owned()))
}

#[macro_export]
macro_rules! suppress_iph {
    ($e:expr) => {{
        #[cfg(all(windows, target_env = "msvc"))]
        {
            let old = $crate::stdlib::os::_set_thread_local_invalid_parameter_handler(
                $crate::stdlib::os::silent_iph_handler,
            );
            let ret = $e;
            $crate::stdlib::os::_set_thread_local_invalid_parameter_handler(old);
            ret
        }
        #[cfg(not(all(windows, target_env = "msvc")))]
        {
            $e
        }
    }};
}

#[allow(dead_code)]
fn os_unimpl<T>(func: &str, vm: &VirtualMachine) -> PyResult<T> {
    Err(vm.new_os_error(format!("{} is not supported on this platform", func)))
}

#[pymodule(name = "os")]
mod _os {
    use super::OpenFlags;
    use super::*;

    #[pyattr]
    use libc::{
        O_APPEND, O_CREAT, O_EXCL, O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY, SEEK_CUR, SEEK_END,
        SEEK_SET,
    };
    #[cfg(not(any(windows, target_os = "wasi")))]
    #[pyattr]
    use libc::{PRIO_PGRP, PRIO_PROCESS, PRIO_USER};
    #[cfg(any(target_os = "dragonfly", target_os = "freebsd", target_os = "linux"))]
    #[pyattr]
    use libc::{SEEK_DATA, SEEK_HOLE};
    #[pyattr]
    pub(super) const F_OK: u8 = 0;
    #[pyattr]
    pub(super) const R_OK: u8 = 4;
    #[pyattr]
    pub(super) const W_OK: u8 = 2;
    #[pyattr]
    pub(super) const X_OK: u8 = 1;

    #[pyfunction]
    fn close(fileno: i64) {
        //The File type automatically closes when it goes out of scope.
        //To enable us to close these file descriptors (and hence prevent leaks)
        //we seek to create the relevant File and simply let it pass out of scope!
        rust_file(fileno);
    }

    #[pyfunction]
    fn closerange(fd_low: i64, fd_high: i64) {
        for fileno in fd_low..fd_high {
            close(fileno);
        }
    }

    #[cfg(any(unix, windows, target_os = "wasi"))]
    #[pyfunction]
    pub(crate) fn open(
        name: PyPathLike,
        flags: OpenFlags,
        _mode: OptionalArg<PyIntRef>,
        dir_fd: DirFd,
        vm: &VirtualMachine,
    ) -> PyResult<i64> {
        let fname = make_path(vm, &name, &dir_fd)?;
        if osstr_contains_nul(&fname) {
            return Err(vm.new_value_error("embedded null character".to_owned()));
        }

        let mut options = OpenOptions::new();

        macro_rules! bit_contains {
            ($c:expr) => {
                flags & $c as OpenFlags == $c as OpenFlags
            };
        }

        if bit_contains!(libc::O_WRONLY) {
            options.write(true);
        } else if bit_contains!(libc::O_RDWR) {
            options.read(true).write(true);
        } else if bit_contains!(libc::O_RDONLY) {
            options.read(true);
        }

        if bit_contains!(libc::O_APPEND) {
            options.append(true);
        }

        if bit_contains!(libc::O_CREAT) {
            if bit_contains!(libc::O_EXCL) {
                options.create_new(true);
            } else {
                options.create(true);
            }
        }

        #[cfg(windows)]
        let flags = flags & !(libc::O_WRONLY as u32);

        #[cfg(not(target_os = "wasi"))]
        {
            use platform::OpenOptionsExt;
            options.custom_flags(flags);
        }
        let handle = options
            .open(fname)
            .map_err(|err| err.into_pyexception(vm))?;

        Ok(raw_file_number(handle))
    }

    #[cfg(not(any(unix, windows, target_os = "wasi")))]
    #[pyfunction]
    pub(crate) fn open(vm: &VirtualMachine, args: FuncArgs) -> PyResult {
        Err(vm.new_os_error("os.open not implemented on this platform".to_owned()))
    }

    #[cfg(any(target_os = "linux"))]
    #[pyfunction]
    fn sendfile(out_fd: i32, in_fd: i32, offset: i64, count: u64, vm: &VirtualMachine) -> PyResult {
        let mut file_offset = offset;

        let res =
            nix::sys::sendfile::sendfile(out_fd, in_fd, Some(&mut file_offset), count as usize)
                .map_err(|err| err.into_pyexception(vm))?;
        Ok(vm.ctx.new_int(res as u64))
    }

    #[cfg(any(target_os = "macos"))]
    #[pyfunction]
    #[allow(clippy::too_many_arguments)]
    fn sendfile(
        out_fd: i32,
        in_fd: i32,
        offset: i64,
        count: i64,
        headers: OptionalArg<PyObjectRef>,
        trailers: OptionalArg<PyObjectRef>,
        _flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let headers = match headers.into_option() {
            Some(x) => Some(vm.extract_elements::<PyBytesLike>(&x)?),
            None => None,
        };

        let headers = headers
            .as_ref()
            .map(|v| v.iter().map(|b| b.borrow_value()).collect::<Vec<_>>());
        let headers = headers
            .as_ref()
            .map(|v| v.iter().map(|borrowed| &**borrowed).collect::<Vec<_>>());
        let headers = headers.as_deref();

        let trailers = match trailers.into_option() {
            Some(x) => Some(vm.extract_elements::<PyBytesLike>(&x)?),
            None => None,
        };

        let trailers = trailers
            .as_ref()
            .map(|v| v.iter().map(|b| b.borrow_value()).collect::<Vec<_>>());
        let trailers = trailers
            .as_ref()
            .map(|v| v.iter().map(|borrowed| &**borrowed).collect::<Vec<_>>());
        let trailers = trailers.as_deref();

        let (res, written) =
            nix::sys::sendfile::sendfile(in_fd, out_fd, offset, Some(count), headers, trailers);
        res.map_err(|err| err.into_pyexception(vm))?;
        Ok(vm.ctx.new_int(written as u64))
    }

    #[pyfunction]
    fn error(message: OptionalArg<PyStrRef>, vm: &VirtualMachine) -> PyResult {
        let msg = message.map_or("".to_owned(), |msg| msg.borrow_value().to_owned());

        Err(vm.new_os_error(msg))
    }

    #[pyfunction]
    fn fsync(fd: i64, vm: &VirtualMachine) -> PyResult<()> {
        let file = rust_file(fd);
        file.sync_all().map_err(|err| err.into_pyexception(vm))?;
        // Avoid closing the fd
        raw_file_number(file);
        Ok(())
    }

    #[pyfunction]
    fn read(fd: i64, n: usize, vm: &VirtualMachine) -> PyResult {
        let mut buffer = vec![0u8; n];
        let mut file = rust_file(fd);
        let n = file
            .read(&mut buffer)
            .map_err(|err| err.into_pyexception(vm))?;
        buffer.truncate(n);

        // Avoid closing the fd
        raw_file_number(file);
        Ok(vm.ctx.new_bytes(buffer))
    }

    #[pyfunction]
    fn write(fd: i64, data: PyBytesLike, vm: &VirtualMachine) -> PyResult {
        let mut file = rust_file(fd);
        let written = data
            .with_ref(|b| file.write(b))
            .map_err(|err| err.into_pyexception(vm))?;

        // Avoid closing the fd
        raw_file_number(file);
        Ok(vm.ctx.new_int(written))
    }

    #[pyfunction]
    fn remove(path: PyPathLike, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult<()> {
        let path = make_path(vm, &path, &dir_fd)?;
        let is_junction = cfg!(windows)
            && fs::symlink_metadata(&path).map_or(false, |meta| {
                let ty = meta.file_type();
                ty.is_dir() && ty.is_symlink()
            });
        let res = if is_junction {
            fs::remove_dir(&path)
        } else {
            fs::remove_file(&path)
        };
        res.map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn mkdir(
        path: PyPathLike,
        _mode: OptionalArg<PyIntRef>,
        dir_fd: DirFd,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let path = make_path(vm, &path, &dir_fd)?;
        fs::create_dir(path).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn mkdirs(path: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        fs::create_dir_all(path.borrow_value()).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn rmdir(path: PyPathLike, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult<()> {
        let path = make_path(vm, &path, &dir_fd)?;
        fs::remove_dir(path).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn listdir(path: Either<PyPathLike, i64>, vm: &VirtualMachine) -> PyResult {
        let path = match path {
            Either::A(path) => path,
            Either::B(fno) => PyPathLike::new_str(path_from_fd(fno, vm)?),
        };
        let dir_iter = fs::read_dir(&path).map_err(|err| err.into_pyexception(vm))?;
        let res: PyResult<Vec<PyObjectRef>> = dir_iter
            .map(|entry| match entry {
                Ok(entry_path) => path.mode.process_path(entry_path.file_name(), vm),
                Err(err) => Err(err.into_pyexception(vm)),
            })
            .collect();
        Ok(vm.ctx.new_list(res?))
    }

    #[pyfunction]
    fn putenv(
        key: Either<PyStrRef, PyBytesRef>,
        value: Either<PyStrRef, PyBytesRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let key: &ffi::OsStr = match key {
            Either::A(ref s) => s.borrow_value().as_ref(),
            Either::B(ref b) => bytes_as_osstr(b.borrow_value(), vm)?,
        };
        let value: &ffi::OsStr = match value {
            Either::A(ref s) => s.borrow_value().as_ref(),
            Either::B(ref b) => bytes_as_osstr(b.borrow_value(), vm)?,
        };
        env::set_var(key, value);
        Ok(())
    }

    #[pyfunction]
    fn unsetenv(key: Either<PyStrRef, PyBytesRef>, vm: &VirtualMachine) -> PyResult<()> {
        let key: &ffi::OsStr = match key {
            Either::A(ref s) => s.borrow_value().as_ref(),
            Either::B(ref b) => bytes_as_osstr(b.borrow_value(), vm)?,
        };
        env::remove_var(key);
        Ok(())
    }

    #[pyfunction]
    fn readlink(path: PyPathLike, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult {
        let mode = path.mode;
        let path = make_path(vm, &path, &dir_fd)?;
        let path = fs::read_link(path).map_err(|err| err.into_pyexception(vm))?;
        mode.process_path(path, vm)
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug)]
    struct DirEntry {
        entry: fs::DirEntry,
        mode: OutputMode,
    }

    impl PyValue for DirEntry {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl DirEntry {
        #[pyproperty]
        fn name(&self, vm: &VirtualMachine) -> PyResult {
            self.mode.process_path(self.entry.file_name(), vm)
        }

        #[pyproperty]
        fn path(&self, vm: &VirtualMachine) -> PyResult {
            self.mode.process_path(self.entry.path(), vm)
        }

        fn perform_on_metadata(
            &self,
            follow_symlinks: FollowSymlinks,
            action: fn(fs::Metadata) -> bool,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            let meta = fs_metadata(self.entry.path(), follow_symlinks.0)
                .map_err(|err| err.into_pyexception(vm))?;
            Ok(action(meta))
        }

        #[pymethod]
        fn is_dir(&self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<bool> {
            self.perform_on_metadata(
                follow_symlinks,
                |meta: fs::Metadata| -> bool { meta.is_dir() },
                vm,
            )
        }

        #[pymethod]
        fn is_file(&self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<bool> {
            self.perform_on_metadata(
                follow_symlinks,
                |meta: fs::Metadata| -> bool { meta.is_file() },
                vm,
            )
        }

        #[pymethod]
        fn is_symlink(&self, vm: &VirtualMachine) -> PyResult<bool> {
            Ok(self
                .entry
                .file_type()
                .map_err(|err| err.into_pyexception(vm))?
                .is_symlink())
        }

        #[pymethod]
        fn stat(
            &self,
            dir_fd: DirFd,
            follow_symlinks: FollowSymlinks,
            vm: &VirtualMachine,
        ) -> PyResult {
            stat(
                Either::A(PyPathLike {
                    path: self.entry.path(),
                    mode: OutputMode::String,
                }),
                dir_fd,
                follow_symlinks,
                vm,
            )
        }

        #[pymethod(magic)]
        fn fspath(&self, vm: &VirtualMachine) -> PyResult {
            self.path(vm)
        }
    }

    #[pyattr]
    #[pyclass(name = "ScandirIter")]
    #[derive(Debug)]
    struct ScandirIterator {
        entries: PyRwLock<fs::ReadDir>,
        exhausted: AtomicCell<bool>,
        mode: OutputMode,
    }

    impl PyValue for ScandirIterator {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(with(PyIter))]
    impl ScandirIterator {
        #[pymethod]
        fn close(&self) {
            self.exhausted.store(true);
        }

        #[pymethod(name = "__enter__")]
        fn enter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod(name = "__exit__")]
        fn exit(zelf: PyRef<Self>, _args: FuncArgs) {
            zelf.close()
        }
    }
    impl PyIter for ScandirIterator {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            if zelf.exhausted.load() {
                return Err(vm.new_stop_iteration());
            }

            match zelf.entries.write().next() {
                Some(entry) => match entry {
                    Ok(entry) => Ok(DirEntry {
                        entry,
                        mode: zelf.mode,
                    }
                    .into_ref(vm)
                    .into_object()),
                    Err(err) => Err(err.into_pyexception(vm)),
                },
                None => {
                    zelf.exhausted.store(true);
                    Err(vm.new_stop_iteration())
                }
            }
        }
    }

    #[pyfunction]
    fn scandir(path: OptionalArg<PyPathLike>, vm: &VirtualMachine) -> PyResult {
        let path = match path {
            OptionalArg::Present(path) => path,
            OptionalArg::Missing => PyPathLike::new_str("."),
        };

        let entries = fs::read_dir(path.path).map_err(|err| err.into_pyexception(vm))?;
        Ok(ScandirIterator {
            entries: PyRwLock::new(entries),
            exhausted: AtomicCell::new(false),
            mode: path.mode,
        }
        .into_ref(vm)
        .into_object())
    }

    #[pyattr]
    #[pyclass(module = "os", name = "stat_result")]
    #[derive(Debug, PyStructSequence)]
    struct StatResult {
        pub st_mode: u32,
        pub st_ino: u64,
        pub st_dev: u64,
        pub st_nlink: u64,
        pub st_uid: u32,
        pub st_gid: u32,
        pub st_size: u64,
        // TODO: unnamed structsequence fields
        pub __st_atime_int: BigInt,
        pub __st_mtime_int: BigInt,
        pub __st_ctime_int: BigInt,
        pub st_atime: f64,
        pub st_mtime: f64,
        pub st_ctime: f64,
        pub st_atime_ns: BigInt,
        pub st_mtime_ns: BigInt,
        pub st_ctime_ns: BigInt,
    }

    #[pyimpl(with(PyStructSequence))]
    impl StatResult {
        fn into_obj(self, vm: &VirtualMachine) -> PyObjectRef {
            self.into_struct_sequence(vm).unwrap().into_object()
        }

        fn from_metadata(meta: fs::Metadata) -> io::Result<Self> {
            let (st_mode, st_ino, st_dev, st_nlink, st_uid, st_gid, ctime);
            #[cfg(windows)]
            {
                ctime = meta.created()?;
                st_ino = 0; // TODO: Not implemented in std::os::windows::fs::MetadataExt.
                st_dev = 0; // TODO: Not implemented in std::os::windows::fs::MetadataExt.
                st_nlink = 0; // TODO: Not implemented in std::os::windows::fs::MetadataExt.
                st_uid = 0; // 0 on windows
                st_gid = 0; // 0 on windows
            }
            #[cfg(unix)]
            {
                #[cfg(target_os = "android")]
                use std::os::android::fs::MetadataExt;
                #[cfg(target_os = "freebsd")]
                use std::os::freebsd::fs::MetadataExt;
                #[cfg(target_os = "linux")]
                use std::os::linux::fs::MetadataExt;
                #[cfg(target_os = "macos")]
                use std::os::macos::fs::MetadataExt;
                #[cfg(target_os = "openbsd")]
                use std::os::openbsd::fs::MetadataExt;
                #[cfg(target_os = "redox")]
                use std::os::redox::fs::MetadataExt;

                // TODO: figure out a better way to do this, SystemTime is just a wrapper over timespec
                ctime = {
                    let (ctime, ctime_ns) = (meta.st_ctime(), meta.st_ctime_nsec());
                    let dur = std::time::Duration::new(ctime.abs() as _, ctime_ns as _);
                    if ctime < 0 {
                        SystemTime::UNIX_EPOCH - dur
                    } else {
                        SystemTime::UNIX_EPOCH + dur
                    }
                };
                st_mode = meta.st_mode();
                st_ino = meta.st_ino();
                st_dev = meta.st_dev();
                st_nlink = meta.st_nlink();
                st_uid = meta.st_uid();
                st_gid = meta.st_gid();
            }
            #[cfg(target_os = "wasi")]
            {
                // XXX: currently meta.created() returns filestat_t.ctim, which is changetime. That
                // might change in the future and meta.created() might return an error
                ctime = meta.created()?;

                // TODO: wasi::MetadataExt once stable
                // use std::os::wasi::fs::MetadataExt;
                // st_ino = meta.ino();
                // st_dev = meta.dev();
                // st_nlink = meta.nlink();
                st_ino = 0;
                st_dev = 0;
                st_nlink = 0;

                st_uid = 0;
                st_gid = 0;
            }
            #[cfg(not(unix))]
            {
                // Based on CPython fileutils.c' attributes_to_mode
                st_mode = {
                    const S_IFDIR: u32 = 0o040000;
                    const S_IFREG: u32 = 0o100000;
                    let mut m: u32 = 0;
                    if meta.is_dir() {
                        m |= S_IFDIR | 0o111; /* IFEXEC for user,group,other */
                    } else {
                        m |= S_IFREG;
                    }
                    if meta.permissions().readonly() {
                        m |= 0o444;
                    } else {
                        m |= 0o666;
                    }
                    m
                };
            }

            let accessed = meta.accessed()?;
            let modified = meta.modified()?;
            Ok(StatResult {
                st_mode,
                st_ino,
                st_dev,
                st_nlink,
                st_uid,
                st_gid,
                st_size: meta.len(),
                __st_atime_int: int_seconds_epoch(accessed),
                __st_mtime_int: int_seconds_epoch(modified),
                __st_ctime_int: int_seconds_epoch(ctime),
                st_atime: to_seconds_from_unix_epoch(accessed),
                st_mtime: to_seconds_from_unix_epoch(modified),
                st_ctime: to_seconds_from_unix_epoch(ctime),
                st_atime_ns: to_nanoseconds_epoch(accessed),
                st_mtime_ns: to_nanoseconds_epoch(modified),
                st_ctime_ns: to_nanoseconds_epoch(ctime),
            })
        }
    }

    #[pyfunction]
    fn stat(
        file: Either<PyPathLike, i64>,
        dir_fd: DirFd,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult {
        let meta = match file {
            Either::A(path) => fs_metadata(make_path(vm, &path, &dir_fd)?, follow_symlinks.0),
            Either::B(fno) => {
                let file = rust_file(fno);
                let res = file.metadata();
                raw_file_number(file);
                res
            }
        };
        meta.and_then(StatResult::from_metadata)
            .map(|stat| stat.into_obj(vm))
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn lstat(file: Either<PyPathLike, i64>, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult {
        stat(file, dir_fd, FollowSymlinks(false), vm)
    }

    fn curdir_inner(vm: &VirtualMachine) -> PyResult<PathBuf> {
        // getcwd (should) return FileNotFoundError if cwd is invalid; on wasi, we never have a
        // valid cwd ;)
        let res = if cfg!(target_os = "wasi") {
            Err(io::ErrorKind::NotFound.into())
        } else {
            env::current_dir()
        };

        res.map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn getcwd(vm: &VirtualMachine) -> PyResult {
        OutputMode::String.process_path(curdir_inner(vm)?, vm)
    }

    #[pyfunction]
    fn getcwdb(vm: &VirtualMachine) -> PyResult {
        OutputMode::Bytes.process_path(curdir_inner(vm)?, vm)
    }

    #[pyfunction]
    fn chdir(path: PyPathLike, vm: &VirtualMachine) -> PyResult<()> {
        env::set_current_dir(&path.path).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn fspath(path: PyPathLike, vm: &VirtualMachine) -> PyResult {
        path.mode.process_path(path.path, vm)
    }

    #[pyfunction]
    fn rename(src: PyPathLike, dst: PyPathLike, vm: &VirtualMachine) -> PyResult<()> {
        fs::rename(src.path, dst.path).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn getpid(vm: &VirtualMachine) -> PyObjectRef {
        let pid = std::process::id();
        vm.ctx.new_int(pid)
    }

    #[pyfunction]
    fn cpu_count(vm: &VirtualMachine) -> PyObjectRef {
        let cpu_count = num_cpus::get();
        vm.ctx.new_int(cpu_count)
    }

    #[pyfunction]
    fn exit(code: i32) {
        std::process::exit(code)
    }

    #[pyfunction]
    fn abort() {
        extern "C" {
            fn abort();
        }
        unsafe { abort() }
    }

    #[pyfunction]
    fn urandom(size: usize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut buf = vec![0u8; size];
        getrandom::getrandom(&mut buf).map_err(|e| match e.raw_os_error() {
            Some(errno) => io::Error::from_raw_os_error(errno).into_pyexception(vm),
            None => vm.new_os_error("Getting random failed".to_owned()),
        })?;
        Ok(buf)
    }

    #[pyfunction]
    pub fn isatty(fd: i32) -> bool {
        unsafe { suppress_iph!(libc::isatty(fd)) != 0 }
    }

    #[pyfunction]
    pub fn lseek(fd: i32, position: Offset, how: i32, vm: &VirtualMachine) -> PyResult<Offset> {
        #[cfg(not(windows))]
        let res = unsafe { suppress_iph!(libc::lseek(fd, position, how)) };
        #[cfg(windows)]
        let res = unsafe {
            use std::os::windows::io::RawHandle;
            use winapi::um::{fileapi, winnt};
            let mut li = winnt::LARGE_INTEGER::default();
            *li.QuadPart_mut() = position;
            let ret = fileapi::SetFilePointer(
                fd as RawHandle,
                li.u().LowPart as _,
                &mut li.u_mut().HighPart,
                how as _,
            );
            if ret == fileapi::INVALID_SET_FILE_POINTER {
                -1
            } else {
                li.u_mut().LowPart = ret;
                *li.QuadPart()
            }
        };
        if res < 0 {
            Err(errno_err(vm))
        } else {
            Ok(res)
        }
    }

    #[pyfunction]
    fn link(src: PyPathLike, dst: PyPathLike, vm: &VirtualMachine) -> PyResult<()> {
        fs::hard_link(src.path, dst.path).map_err(|err| err.into_pyexception(vm))
    }

    #[derive(FromArgs)]
    struct UtimeArgs {
        #[pyarg(any)]
        path: PyPathLike,
        #[pyarg(any, default)]
        times: Option<PyTupleRef>,
        #[pyarg(named, default)]
        ns: Option<PyTupleRef>,
        #[pyarg(flatten)]
        dir_fd: DirFd,
        #[pyarg(flatten)]
        follow_symlinks: FollowSymlinks,
    }

    #[cfg(not(target_os = "wasi"))]
    #[pyfunction]
    fn utime(args: UtimeArgs, vm: &VirtualMachine) -> PyResult<()> {
        let parse_tup = |tup: &PyTuple| -> Option<(PyObjectRef, PyObjectRef)> {
            let tup = tup.borrow_value();
            if tup.len() != 2 {
                None
            } else {
                Some((tup[0].clone(), tup[1].clone()))
            }
        };
        let (acc, modif) = match (args.times, args.ns) {
            (Some(t), None) => {
                let (a, m) = parse_tup(&t).ok_or_else(|| {
                    vm.new_type_error(
                        "utime: 'times' must be either a tuple of two ints or None".to_owned(),
                    )
                })?;
                (
                    Duration::try_from_object(vm, a)?,
                    Duration::try_from_object(vm, m)?,
                )
            }
            (None, Some(ns)) => {
                let (a, m) = parse_tup(&ns).ok_or_else(|| {
                    vm.new_type_error("utime: 'ns' must be a tuple of two ints".to_owned())
                })?;
                let ns_in_sec = vm.ctx.new_int(1_000_000_000);
                let ns_to_dur = |obj: PyObjectRef| {
                    let divmod = vm._divmod(&obj, &ns_in_sec)?;
                    let (div, rem) =
                        divmod
                            .payload::<PyTuple>()
                            .and_then(parse_tup)
                            .ok_or_else(|| {
                                vm.new_type_error(format!(
                                    "{}.__divmod__() must return a 2-tuple, not {}",
                                    obj.class().name,
                                    divmod.class().name
                                ))
                            })?;
                    let secs = int::try_to_primitive(vm.to_index(&div)?.borrow_value(), vm)?;
                    let ns = int::try_to_primitive(vm.to_index(&rem)?.borrow_value(), vm)?;
                    Ok(Duration::new(secs, ns))
                };
                // TODO: do validation to make sure this doesn't.. underflow?
                (ns_to_dur(a)?, ns_to_dur(m)?)
            }
            (None, None) => {
                let now = SystemTime::now();
                let now = now.duration_since(SystemTime::UNIX_EPOCH).unwrap();
                (now, now)
            }
            (Some(_), Some(_)) => {
                return Err(vm.new_value_error(
                    "utime: you may specify either 'times' or 'ns' but not both".to_owned(),
                ))
            }
        };
        utime_impl(
            args.path.as_ref(),
            acc,
            modif,
            args.dir_fd,
            args.follow_symlinks,
            vm,
        )
    }

    #[cfg(not(target_os = "wasi"))]
    fn utime_impl(
        path: &Path,
        acc: Duration,
        modif: Duration,
        _dir_fd: DirFd,
        _follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        #[cfg(unix)]
        {
            #[cfg(not(target_os = "redox"))]
            {
                nix::sys::stat::utimensat(
                    _dir_fd.0,
                    path,
                    &acc.into(),
                    &modif.into(),
                    if _follow_symlinks.0 {
                        nix::sys::stat::UtimensatFlags::FollowSymlink
                    } else {
                        nix::sys::stat::UtimensatFlags::NoFollowSymlink
                    },
                )
                .map_err(|err| err.into_pyexception(vm))
            }
            #[cfg(target_os = "redox")]
            {
                let tv = |d: Duration| libc::timeval {
                    tv_sec: d.as_secs() as _,
                    tv_usec: d.as_micros() as _,
                };
                nix::sys::stat::utimes(path, &tv(acc).into(), &tv(modif).into())
                    .map_err(|err| err.into_pyexception(vm))
            }
        }
        #[cfg(windows)]
        {
            use std::fs::OpenOptions;
            use std::os::windows::prelude::*;
            use winapi::shared::minwindef::{DWORD, FILETIME};
            use winapi::um::fileapi::SetFileTime;

            let ft = |d: Duration| {
                let intervals =
                    ((d.as_secs() as i64 + 11644473600) * 10_000_000) + (d.as_nanos() as i64 / 100);
                FILETIME {
                    dwLowDateTime: intervals as DWORD,
                    dwHighDateTime: (intervals >> 32) as DWORD,
                }
            };

            let acc = ft(acc);
            let modif = ft(modif);

            let f = OpenOptions::new()
                .write(true)
                .custom_flags(winapi::um::winbase::FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)
                .map_err(|err| err.into_pyexception(vm))?;

            let ret =
                unsafe { SetFileTime(f.as_raw_handle() as _, std::ptr::null(), &acc, &modif) };

            if ret == 0 {
                Err(io::Error::last_os_error().into_pyexception(vm))
            } else {
                Ok(())
            }
        }
    }

    #[pyfunction]
    fn strerror(e: i32) -> String {
        unsafe { ffi::CStr::from_ptr(libc::strerror(e)) }
            .to_string_lossy()
            .into_owned()
    }

    #[pyfunction]
    pub fn ftruncate(fd: i64, length: Offset, vm: &VirtualMachine) -> PyResult<()> {
        let f = rust_file(fd);
        f.set_len(length as u64)
            .map_err(|e| e.into_pyexception(vm))?;
        raw_file_number(f);
        Ok(())
    }

    #[pyfunction]
    fn truncate(path: PyObjectRef, length: Offset, vm: &VirtualMachine) -> PyResult<()> {
        if let Ok(fd) = i64::try_from_object(vm, path.clone()) {
            return ftruncate(fd, length, vm);
        }
        let path = PyPathLike::try_from_object(vm, path)?;
        // TODO: just call libc::truncate() on POSIX
        let f = OpenOptions::new()
            .write(true)
            .open(&path)
            .map_err(|e| e.into_pyexception(vm))?;
        f.set_len(length as u64)
            .map_err(|e| e.into_pyexception(vm))?;
        drop(f);
        Ok(())
    }

    #[pyattr]
    #[pyclass(module = "os", name = "terminal_size")]
    #[derive(PyStructSequence)]
    #[allow(dead_code)]
    pub(super) struct PyTerminalSize {
        pub columns: usize,
        pub lines: usize,
    }
    #[pyimpl(with(PyStructSequence))]
    impl PyTerminalSize {}

    pub(super) fn support_funcs(vm: &VirtualMachine) -> Vec<SupportFunc> {
        let mut supports = super::platform::support_funcs(vm);
        supports.extend(vec![
            SupportFunc::new(vm, "open", open, None, Some(true), None),
            SupportFunc::new(
                vm,
                "access",
                platform::access,
                Some(false),
                Some(false),
                None,
            ),
            SupportFunc::new(vm, "chdir", chdir, Some(false), None, None),
            // chflags Some, None Some
            SupportFunc::new(vm, "listdir", listdir, Some(true), None, None),
            SupportFunc::new(vm, "mkdir", mkdir, Some(false), Some(false), None),
            // mkfifo Some Some None
            // mknod Some Some None
            // pathconf Some None None
            SupportFunc::new(vm, "readlink", readlink, Some(false), Some(false), None),
            SupportFunc::new(vm, "remove", remove, Some(false), Some(false), None),
            SupportFunc::new(vm, "rename", rename, Some(false), Some(false), None),
            SupportFunc::new(vm, "replace", rename, Some(false), Some(false), None), // TODO: Fix replace
            SupportFunc::new(vm, "rmdir", rmdir, Some(false), Some(false), None),
            SupportFunc::new(vm, "scandir", scandir, Some(false), None, None),
            SupportFunc::new(vm, "stat", stat, Some(true), Some(true), Some(true)),
            SupportFunc::new(vm, "fstat", stat, Some(true), Some(true), Some(true)),
            SupportFunc::new(vm, "symlink", platform::symlink, None, Some(false), None),
            // truncate Some None None
            SupportFunc::new(vm, "unlink", remove, Some(false), Some(false), None),
            #[cfg(not(target_os = "wasi"))]
            SupportFunc::new(
                vm,
                "utime",
                utime,
                Some(false),
                Some(cfg!(all(unix, not(target_os = "redox")))),
                Some(cfg!(all(unix, not(target_os = "redox")))),
            ),
        ]);
        supports
    }
}
pub(crate) use _os::{ftruncate, isatty, lseek};

struct SupportFunc {
    name: &'static str,
    func_obj: PyObjectRef,
    fd: Option<bool>,
    dir_fd: Option<bool>,
    follow_symlinks: Option<bool>,
}

impl<'a> SupportFunc {
    fn new<F, FKind>(
        vm: &VirtualMachine,
        name: &'static str,
        func: F,
        fd: Option<bool>,
        dir_fd: Option<bool>,
        follow_symlinks: Option<bool>,
    ) -> Self
    where
        F: IntoPyNativeFunc<FKind>,
    {
        let ctx = &vm.ctx;
        let func_obj = ctx
            .make_funcdef(name, func)
            .into_function()
            .with_module(ctx.new_str(MODULE_NAME))
            .build(ctx);
        Self {
            name,
            func_obj,
            fd,
            dir_fd,
            follow_symlinks,
        }
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = platform::make_module(vm);

    _os::extend_module(&vm, &module);

    let support_funcs = _os::support_funcs(vm);
    let supports_fd = PySet::default().into_ref(vm);
    let supports_dir_fd = PySet::default().into_ref(vm);
    let supports_follow_symlinks = PySet::default().into_ref(vm);
    for support in support_funcs {
        if support.fd.unwrap_or(false) {
            supports_fd
                .clone()
                .add(support.func_obj.clone(), vm)
                .unwrap();
        }
        if support.dir_fd.unwrap_or(false) {
            supports_dir_fd
                .clone()
                .add(support.func_obj.clone(), vm)
                .unwrap();
        }
        if support.follow_symlinks.unwrap_or(false) {
            supports_follow_symlinks
                .clone()
                .add(support.func_obj.clone(), vm)
                .unwrap();
        }
        vm.set_attr(&module, support.name, support.func_obj)
            .unwrap();
    }

    extend_module!(vm, module, {
        "supports_fd" => supports_fd.into_object(),
        "supports_dir_fd" => supports_dir_fd.into_object(),
        "supports_follow_symlinks" => supports_follow_symlinks.into_object(),
    });

    module
}
pub(crate) use _os::open;

fn to_seconds_from_unix_epoch(sys_time: SystemTime) -> f64 {
    match sys_time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs_f64(),
        Err(err) => -err.duration().as_secs_f64(),
    }
}

fn to_nanoseconds_epoch(sys_time: SystemTime) -> BigInt {
    match sys_time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => BigInt::from(duration.as_nanos()),
        Err(err) => -BigInt::from(err.duration().as_nanos()),
    }
}

fn int_seconds_epoch(sys_time: SystemTime) -> BigInt {
    match sys_time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => BigInt::from(duration.as_secs()),
        Err(err) => -BigInt::from(err.duration().as_secs()),
    }
}

#[cfg(unix)]
#[pymodule]
mod posix {
    use super::*;

    use crate::builtins::dict::PyMapping;
    use crate::builtins::list::PyListRef;
    use crate::pyobject::PyIterable;
    use bitflags::bitflags;
    use nix::errno::{errno, Errno};
    use nix::unistd::{self, Gid, Pid, Uid};
    use std::convert::TryFrom;
    pub(super) use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::RawFd;

    #[cfg(not(any(target_os = "redox", target_os = "freebsd")))]
    #[pyattr]
    use libc::O_DSYNC;
    #[pyattr]
    use libc::{O_CLOEXEC, O_NONBLOCK, WNOHANG};
    #[cfg(not(target_os = "redox"))]
    #[pyattr]
    use libc::{O_NDELAY, O_NOCTTY};

    #[pyattr]
    const EX_OK: i8 = exitcode::OK as i8;
    #[pyattr]
    const EX_USAGE: i8 = exitcode::USAGE as i8;
    #[pyattr]
    const EX_DATAERR: i8 = exitcode::DATAERR as i8;
    #[pyattr]
    const EX_NOINPUT: i8 = exitcode::NOINPUT as i8;
    #[pyattr]
    const EX_NOUSER: i8 = exitcode::NOUSER as i8;
    #[pyattr]
    const EX_NOHOST: i8 = exitcode::NOHOST as i8;
    #[pyattr]
    const EX_UNAVAILABLE: i8 = exitcode::UNAVAILABLE as i8;
    #[pyattr]
    const EX_SOFTWARE: i8 = exitcode::SOFTWARE as i8;
    #[pyattr]
    const EX_OSERR: i8 = exitcode::OSERR as i8;
    #[pyattr]
    const EX_OSFILE: i8 = exitcode::OSFILE as i8;
    #[pyattr]
    const EX_CANTCREAT: i8 = exitcode::CANTCREAT as i8;
    #[pyattr]
    const EX_IOERR: i8 = exitcode::IOERR as i8;
    #[pyattr]
    const EX_TEMPFAIL: i8 = exitcode::TEMPFAIL as i8;
    #[pyattr]
    const EX_PROTOCOL: i8 = exitcode::PROTOCOL as i8;
    #[pyattr]
    const EX_NOPERM: i8 = exitcode::NOPERM as i8;
    #[pyattr]
    const EX_CONFIG: i8 = exitcode::CONFIG as i8;

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    const POSIX_SPAWN_OPEN: i32 = PosixSpawnFileActionIdentifier::Open as i32;
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    const POSIX_SPAWN_CLOSE: i32 = PosixSpawnFileActionIdentifier::Close as i32;
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    const POSIX_SPAWN_DUP2: i32 = PosixSpawnFileActionIdentifier::Dup2 as i32;

    #[cfg(target_os = "macos")]
    #[pyattr]
    const _COPYFILE_DATA: u32 = 1 << 3;

    pub(crate) type OpenFlags = i32;

    // Flags for os_access
    bitflags! {
        pub struct AccessFlags: u8{
            const F_OK = super::_os::F_OK;
            const R_OK = super::_os::R_OK;
            const W_OK = super::_os::W_OK;
            const X_OK = super::_os::X_OK;
        }
    }

    impl PyPathLike {
        pub fn into_bytes(self) -> Vec<u8> {
            use std::os::unix::ffi::OsStringExt;
            self.path.into_os_string().into_vec()
        }
    }

    pub(crate) fn raw_file_number(handle: File) -> i64 {
        use std::os::unix::io::IntoRawFd;

        i64::from(handle.into_raw_fd())
    }

    pub(crate) fn rust_file(raw_fileno: i64) -> File {
        use std::os::unix::io::FromRawFd;

        unsafe { File::from_raw_fd(raw_fileno as i32) }
    }

    pub(super) fn convert_nix_errno(vm: &VirtualMachine, errno: Errno) -> PyTypeRef {
        match errno {
            Errno::EPERM => vm.ctx.exceptions.permission_error.clone(),
            _ => vm.ctx.exceptions.os_error.clone(),
        }
    }

    struct Permissions {
        is_readable: bool,
        is_writable: bool,
        is_executable: bool,
    }

    fn get_permissions(mode: u32) -> Permissions {
        Permissions {
            is_readable: mode & 4 != 0,
            is_writable: mode & 2 != 0,
            is_executable: mode & 1 != 0,
        }
    }

    fn get_right_permission(
        mode: u32,
        file_owner: Uid,
        file_group: Gid,
    ) -> nix::Result<Permissions> {
        let owner_mode = (mode & 0o700) >> 6;
        let owner_permissions = get_permissions(owner_mode);

        let group_mode = (mode & 0o070) >> 3;
        let group_permissions = get_permissions(group_mode);

        let others_mode = mode & 0o007;
        let others_permissions = get_permissions(others_mode);

        let user_id = nix::unistd::getuid();
        let groups_ids = getgroups()?;

        if file_owner == user_id {
            Ok(owner_permissions)
        } else if groups_ids.contains(&file_group) {
            Ok(group_permissions)
        } else {
            Ok(others_permissions)
        }
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    fn getgroups() -> nix::Result<Vec<Gid>> {
        use libc::{c_int, gid_t};
        use std::ptr;
        let ret = unsafe { libc::getgroups(0, ptr::null_mut()) };
        let mut groups = Vec::<Gid>::with_capacity(Errno::result(ret)? as usize);
        let ret = unsafe {
            libc::getgroups(
                groups.capacity() as c_int,
                groups.as_mut_ptr() as *mut gid_t,
            )
        };

        Errno::result(ret).map(|s| {
            unsafe { groups.set_len(s as usize) };
            groups
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "redox")))]
    use nix::unistd::getgroups;

    #[cfg(target_os = "redox")]
    fn getgroups() -> nix::Result<Vec<Gid>> {
        Err(nix::Error::UnsupportedOperation)
    }

    #[pyfunction]
    pub(super) fn access(path: PyPathLike, mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        use std::os::unix::fs::MetadataExt;

        let flags = AccessFlags::from_bits(mode).ok_or_else(|| {
            vm.new_value_error(
            "One of the flags is wrong, there are only 4 possibilities F_OK, R_OK, W_OK and X_OK"
                .to_owned(),
        )
        })?;

        let metadata = fs::metadata(&path.path);

        // if it's only checking for F_OK
        if flags == AccessFlags::F_OK {
            return Ok(metadata.is_ok());
        }

        let metadata = metadata.map_err(|err| err.into_pyexception(vm))?;

        let user_id = metadata.uid();
        let group_id = metadata.gid();
        let mode = metadata.mode();

        let perm = get_right_permission(mode, Uid::from_raw(user_id), Gid::from_raw(group_id))
            .map_err(|err| err.into_pyexception(vm))?;

        let r_ok = !flags.contains(AccessFlags::R_OK) || perm.is_readable;
        let w_ok = !flags.contains(AccessFlags::W_OK) || perm.is_writable;
        let x_ok = !flags.contains(AccessFlags::X_OK) || perm.is_executable;

        Ok(r_ok && w_ok && x_ok)
    }

    pub(super) fn bytes_as_osstr<'a>(
        b: &'a [u8],
        _vm: &VirtualMachine,
    ) -> PyResult<&'a ffi::OsStr> {
        use std::os::unix::ffi::OsStrExt;
        Ok(ffi::OsStr::from_bytes(b))
    }

    #[pyattr]
    fn environ(vm: &VirtualMachine) -> PyDictRef {
        let environ = vm.ctx.new_dict();
        use std::os::unix::ffi::OsStringExt;
        for (key, value) in env::vars_os() {
            environ
                .set_item(
                    vm.ctx.new_bytes(key.into_vec()),
                    vm.ctx.new_bytes(value.into_vec()),
                    vm,
                )
                .unwrap();
        }

        environ
    }

    #[pyfunction]
    pub(super) fn symlink(
        src: PyPathLike,
        dst: PyPathLike,
        _target_is_directory: TargetIsDirectory,
        dir_fd: DirFd,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        use std::os::unix::fs as unix_fs;
        let dst = make_path(vm, &dst, &dir_fd)?;
        unix_fs::symlink(src.path, dst).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn chroot(path: PyPathLike, vm: &VirtualMachine) -> PyResult<()> {
        nix::unistd::chroot(&*path.path).map_err(|err| err.into_pyexception(vm))
    }

    // As of now, redox does not seems to support chown command (cf. https://gitlab.redox-os.org/redox-os/coreutils , last checked on 05/07/2020)
    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn chown(
        path: Either<PyPathLike, i64>,
        uid: PyIntRef,
        gid: PyIntRef,
        dir_fd: DirFd,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let uid = isize::try_from_object(&vm, uid.as_object().clone())?;
        let gid = isize::try_from_object(&vm, gid.as_object().clone())?;

        let uid = if uid >= 0 {
            Some(nix::unistd::Uid::from_raw(uid as u32))
        } else if uid == -1 {
            None
        } else {
            return Err(vm.new_os_error(String::from("Specified uid is not valid.")));
        };

        let gid = if gid >= 0 {
            Some(nix::unistd::Gid::from_raw(gid as u32))
        } else if gid == -1 {
            None
        } else {
            return Err(vm.new_os_error(String::from("Specified gid is not valid.")));
        };

        let flag = if follow_symlinks.0 {
            nix::unistd::FchownatFlags::FollowSymlink
        } else {
            nix::unistd::FchownatFlags::NoFollowSymlink
        };

        match path {
            Either::A(p) => nix::unistd::fchownat(dir_fd.0, p.path.as_os_str(), uid, gid, flag),
            Either::B(fd) => {
                let path = fs::read_link(format!("/proc/self/fd/{}", fd)).map_err(|_| {
                    vm.new_os_error(String::from("Cannot find path for specified fd"))
                })?;
                nix::unistd::fchownat(dir_fd.0, &path, uid, gid, flag)
            }
        }
        .map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn lchown(path: PyPathLike, uid: PyIntRef, gid: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        chown(
            Either::A(path),
            uid,
            gid,
            DirFd(None),
            FollowSymlinks(false),
            vm,
        )
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn fchown(fd: i64, uid: PyIntRef, gid: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        chown(
            Either::B(fd),
            uid,
            gid,
            DirFd(None),
            FollowSymlinks(true),
            vm,
        )
    }

    #[pyfunction]
    fn get_inheritable(fd: RawFd, vm: &VirtualMachine) -> PyResult<bool> {
        use nix::fcntl::fcntl;
        use nix::fcntl::FcntlArg;
        let flags = fcntl(fd, FcntlArg::F_GETFD);
        match flags {
            Ok(ret) => Ok((ret & libc::FD_CLOEXEC) == 0),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    pub(crate) fn raw_set_inheritable(fd: RawFd, inheritable: bool) -> nix::Result<()> {
        use nix::fcntl;
        let flags = fcntl::FdFlag::from_bits_truncate(fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFD)?);
        let mut new_flags = flags;
        new_flags.set(fcntl::FdFlag::FD_CLOEXEC, !inheritable);
        if flags != new_flags {
            fcntl::fcntl(fd, fcntl::FcntlArg::F_SETFD(new_flags))?;
        }
        Ok(())
    }

    #[pyfunction]
    fn set_inheritable(fd: i64, inheritable: bool, vm: &VirtualMachine) -> PyResult<()> {
        raw_set_inheritable(fd as RawFd, inheritable).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn get_blocking(fd: RawFd, vm: &VirtualMachine) -> PyResult<bool> {
        use nix::fcntl::fcntl;
        use nix::fcntl::FcntlArg;
        let flags = fcntl(fd, FcntlArg::F_GETFL);
        match flags {
            Ok(ret) => Ok((ret & libc::O_NONBLOCK) == 0),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    #[pyfunction]
    fn set_blocking(fd: RawFd, blocking: bool, vm: &VirtualMachine) -> PyResult<()> {
        let _set_flag = || {
            use nix::fcntl::fcntl;
            use nix::fcntl::FcntlArg;
            use nix::fcntl::OFlag;

            let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL)?);
            let mut new_flags = flags;
            new_flags.set(OFlag::from_bits_truncate(libc::O_NONBLOCK), !blocking);
            if flags != new_flags {
                fcntl(fd, FcntlArg::F_SETFL(new_flags))?;
            }
            Ok(())
        };
        _set_flag().map_err(|err: nix::Error| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn pipe(vm: &VirtualMachine) -> PyResult<(RawFd, RawFd)> {
        use nix::unistd::close;
        use nix::unistd::pipe;
        let (rfd, wfd) = pipe().map_err(|err| err.into_pyexception(vm))?;
        set_inheritable(rfd.into(), false, vm)
            .and_then(|_| set_inheritable(wfd.into(), false, vm))
            .map_err(|err| {
                let _ = close(rfd);
                let _ = close(wfd);
                err
            })?;
        Ok((rfd, wfd))
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "emscripten",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn pipe2(flags: libc::c_int, vm: &VirtualMachine) -> PyResult<(RawFd, RawFd)> {
        use nix::fcntl::OFlag;
        use nix::unistd::pipe2;
        let oflags = OFlag::from_bits_truncate(flags);
        pipe2(oflags).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn system(command: PyStrRef) -> PyResult<i32> {
        use std::ffi::CString;

        let rstr = command.borrow_value();
        let cstr = CString::new(rstr).unwrap();
        let x = unsafe { libc::system(cstr.as_ptr()) };
        Ok(x)
    }

    #[pyfunction]
    fn chmod(
        path: PyPathLike,
        dir_fd: DirFd,
        mode: u32,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let path = make_path(vm, &path, &dir_fd)?;
        let body = move || {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs_metadata(&path, follow_symlinks.0)?;
            let mut permissions = meta.permissions();
            permissions.set_mode(mode);
            fs::set_permissions(&path, permissions)
        };
        body().map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn execv(
        path: PyStrRef,
        argv: Either<PyListRef, PyTupleRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let path = ffi::CString::new(path.borrow_value())
            .map_err(|_| vm.new_value_error("embedded null character".to_owned()))?;

        let argv: Vec<ffi::CString> = vm.extract_elements(argv.as_object())?;
        let argv: Vec<&ffi::CStr> = argv.iter().map(|entry| entry.as_c_str()).collect();

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("execv() arg 2 must not be empty".to_owned()))?;
        if first.to_bytes().is_empty() {
            return Err(
                vm.new_value_error("execv() arg 2 first element cannot be empty".to_owned())
            );
        }

        unistd::execv(&path, &argv)
            .map(|_ok| ())
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn execve(
        path: PyPathLike,
        argv: Either<PyListRef, PyTupleRef>,
        env: PyDictRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let path = ffi::CString::new(path.into_bytes())
            .map_err(|_| vm.new_value_error("embedded null character".to_owned()))?;

        let argv: Vec<ffi::CString> = vm.extract_elements(argv.as_object())?;
        let argv: Vec<&ffi::CStr> = argv.iter().map(|entry| entry.as_c_str()).collect();

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("execve() arg 2 must not be empty".to_owned()))?;

        if first.to_bytes().is_empty() {
            return Err(
                vm.new_value_error("execve() arg 2 first element cannot be empty".to_owned())
            );
        }

        let env = env
            .into_iter()
            .map(|(k, v)| -> PyResult<_> {
                let (key, value) = (
                    PyPathLike::try_from_object(&vm, k)?,
                    PyPathLike::try_from_object(&vm, v)?,
                );

                if key.path.display().to_string().contains('=') {
                    return Err(vm.new_value_error("illegal environment variable name".to_owned()));
                }

                ffi::CString::new(format!("{}={}", key.path.display(), value.path.display()))
                    .map_err(|_| vm.new_value_error("embedded null character".to_owned()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let env: Vec<&ffi::CStr> = env.iter().map(|entry| entry.as_c_str()).collect();

        unistd::execve(&path, &argv, &env).map_err(|err| err.into_pyexception(vm))?;
        Ok(())
    }

    #[pyfunction]
    fn getppid(vm: &VirtualMachine) -> PyObjectRef {
        let ppid = unistd::getppid().as_raw();
        vm.ctx.new_int(ppid)
    }

    #[pyfunction]
    fn getgid(vm: &VirtualMachine) -> PyObjectRef {
        let gid = unistd::getgid().as_raw();
        vm.ctx.new_int(gid)
    }

    #[pyfunction]
    fn getegid(vm: &VirtualMachine) -> PyObjectRef {
        let egid = unistd::getegid().as_raw();
        vm.ctx.new_int(egid)
    }

    #[pyfunction]
    fn getpgid(pid: u32, vm: &VirtualMachine) -> PyResult {
        match unistd::getpgid(Some(Pid::from_raw(pid as i32))) {
            Ok(pgid) => Ok(vm.ctx.new_int(pgid.as_raw())),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    #[pyfunction]
    fn getpgrp(vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_int(unistd::getpgrp().as_raw()))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn getsid(pid: u32, vm: &VirtualMachine) -> PyResult {
        match unistd::getsid(Some(Pid::from_raw(pid as i32))) {
            Ok(sid) => Ok(vm.ctx.new_int(sid.as_raw())),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    #[pyfunction]
    fn getuid(vm: &VirtualMachine) -> PyObjectRef {
        let uid = unistd::getuid().as_raw();
        vm.ctx.new_int(uid)
    }

    #[pyfunction]
    fn geteuid(vm: &VirtualMachine) -> PyObjectRef {
        let euid = unistd::geteuid().as_raw();
        vm.ctx.new_int(euid)
    }

    #[pyfunction]
    fn setgid(gid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setgid(Gid::from_raw(gid)).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn setegid(egid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setegid(Gid::from_raw(egid)).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn setpgid(pid: u32, pgid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setpgid(Pid::from_raw(pid as i32), Pid::from_raw(pgid as i32))
            .map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn setsid(vm: &VirtualMachine) -> PyResult<()> {
        unistd::setsid()
            .map(|_ok| ())
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn setuid(uid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setuid(Uid::from_raw(uid)).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn seteuid(euid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::seteuid(Uid::from_raw(euid)).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn setreuid(ruid: u32, euid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setuid(Uid::from_raw(ruid)).map_err(|err| err.into_pyexception(vm))?;
        unistd::seteuid(Uid::from_raw(euid)).map_err(|err| err.into_pyexception(vm))
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn setresuid(ruid: u32, euid: u32, suid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setresuid(
            Uid::from_raw(ruid),
            Uid::from_raw(euid),
            Uid::from_raw(suid),
        )
        .map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn openpty(vm: &VirtualMachine) -> PyResult {
        let r = nix::pty::openpty(None, None).map_err(|err| err.into_pyexception(vm))?;
        Ok(vm
            .ctx
            .new_tuple(vec![vm.ctx.new_int(r.master), vm.ctx.new_int(r.slave)]))
    }

    #[pyfunction]
    fn ttyname(fd: i32, vm: &VirtualMachine) -> PyResult {
        let name = unsafe { libc::ttyname(fd) };
        if name.is_null() {
            Err(errno_err(vm))
        } else {
            let name = unsafe { ffi::CStr::from_ptr(name) }.to_str().unwrap();
            Ok(vm.ctx.new_str(name))
        }
    }

    #[pyfunction]
    fn umask(mask: libc::mode_t) -> libc::mode_t {
        unsafe { libc::umask(mask) }
    }

    #[pyattr]
    #[pyclass(module = "os", name = "uname_result")]
    #[derive(Debug, PyStructSequence)]
    struct UnameResult {
        sysname: String,
        nodename: String,
        release: String,
        version: String,
        machine: String,
    }

    #[pyimpl(with(PyStructSequence))]
    impl UnameResult {
        fn into_obj(self, vm: &VirtualMachine) -> PyObjectRef {
            self.into_struct_sequence(vm).unwrap().into_object()
        }
    }

    #[pyfunction]
    fn uname(vm: &VirtualMachine) -> PyResult {
        let info = uname::uname().map_err(|err| err.into_pyexception(vm))?;
        Ok(UnameResult {
            sysname: info.sysname,
            nodename: info.nodename,
            release: info.release,
            version: info.version,
            machine: info.machine,
        }
        .into_obj(vm))
    }

    #[pyfunction]
    fn sync() {
        #[cfg(not(any(target_os = "redox", target_os = "android")))]
        unsafe {
            libc::sync();
        }
    }

    // cfg from nix
    #[cfg(any(target_os = "android", target_os = "linux", target_os = "openbsd"))]
    #[pyfunction]
    fn getresuid(vm: &VirtualMachine) -> PyResult<(u32, u32, u32)> {
        let mut ruid = 0;
        let mut euid = 0;
        let mut suid = 0;
        let ret = unsafe { libc::getresuid(&mut ruid, &mut euid, &mut suid) };
        if ret == 0 {
            Ok((ruid, euid, suid))
        } else {
            Err(errno_err(vm))
        }
    }

    // cfg from nix
    #[cfg(any(target_os = "android", target_os = "linux", target_os = "openbsd"))]
    #[pyfunction]
    fn getresgid(vm: &VirtualMachine) -> PyResult<(u32, u32, u32)> {
        let mut rgid = 0;
        let mut egid = 0;
        let mut sgid = 0;
        let ret = unsafe { libc::getresgid(&mut rgid, &mut egid, &mut sgid) };
        if ret == 0 {
            Ok((rgid, egid, sgid))
        } else {
            Err(errno_err(vm))
        }
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn setresgid(rgid: u32, egid: u32, sgid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setresgid(
            Gid::from_raw(rgid),
            Gid::from_raw(egid),
            Gid::from_raw(sgid),
        )
        .map_err(|err| err.into_pyexception(vm))
    }

    // cfg from nix
    #[cfg(any(target_os = "android", target_os = "linux", target_os = "openbsd"))]
    #[pyfunction]
    fn setregid(rgid: u32, egid: u32, vm: &VirtualMachine) -> PyResult<()> {
        let ret = unsafe { libc::setregid(rgid, egid) };
        if ret == 0 {
            Ok(())
        } else {
            Err(errno_err(vm))
        }
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn initgroups(user_name: PyStrRef, gid: u32, vm: &VirtualMachine) -> PyResult<()> {
        let user = ffi::CString::new(user_name.borrow_value()).unwrap();
        let gid = Gid::from_raw(gid);
        unistd::initgroups(&user, gid).map_err(|err| err.into_pyexception(vm))
    }

    // cfg from nix
    #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "redox")))]
    #[pyfunction]
    fn setgroups(group_ids: PyIterable<u32>, vm: &VirtualMachine) -> PyResult<()> {
        let gids = group_ids
            .iter(vm)?
            .map(|entry| match entry {
                Ok(id) => Ok(unistd::Gid::from_raw(id)),
                Err(err) => Err(err),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let ret = unistd::setgroups(&gids);
        ret.map_err(|err| err.into_pyexception(vm))
    }

    fn envp_from_dict(dict: PyDictRef, vm: &VirtualMachine) -> PyResult<Vec<ffi::CString>> {
        dict.into_iter()
            .map(|(k, v)| {
                let k = PyPathLike::try_from_object(vm, k)?.into_bytes();
                let v = PyPathLike::try_from_object(vm, v)?.into_bytes();
                if k.contains(&0) {
                    return Err(
                        vm.new_value_error("envp dict key cannot contain a nul byte".to_owned())
                    );
                }
                if k.contains(&b'=') {
                    return Err(vm.new_value_error(
                        "envp dict key cannot contain a '=' character".to_owned(),
                    ));
                }
                if v.contains(&0) {
                    return Err(
                        vm.new_value_error("envp dict value cannot contain a nul byte".to_owned())
                    );
                }
                let mut env = k;
                env.push(b'=');
                env.extend(v);
                Ok(unsafe { ffi::CString::from_vec_unchecked(env) })
            })
            .collect()
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[derive(FromArgs)]
    pub(super) struct PosixSpawnArgs {
        #[pyarg(positional)]
        path: PyPathLike,
        #[pyarg(positional)]
        args: PyIterable<PyPathLike>,
        #[pyarg(positional)]
        env: PyMapping,
        #[pyarg(named, default)]
        file_actions: Option<PyIterable<PyTupleRef>>,
        #[pyarg(named, default)]
        setsigdef: Option<PyIterable<i32>>,
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
    #[repr(i32)]
    enum PosixSpawnFileActionIdentifier {
        Open,
        Close,
        Dup2,
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    impl PosixSpawnArgs {
        fn spawn(self, spawnp: bool, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
            let path = ffi::CString::new(self.path.into_bytes())
                .map_err(|_| vm.new_value_error("path should not have nul bytes".to_owned()))?;

            let mut file_actions = unsafe {
                let mut fa = std::mem::MaybeUninit::uninit();
                assert!(libc::posix_spawn_file_actions_init(fa.as_mut_ptr()) == 0);
                fa.assume_init()
            };
            if let Some(it) = self.file_actions {
                for action in it.iter(vm)? {
                    let action = action?;
                    let (id, args) = action.borrow_value().split_first().ok_or_else(|| {
                        vm.new_type_error(
                            "Each file_actions element must be a non-empty tuple".to_owned(),
                        )
                    })?;
                    let id = i32::try_from_object(vm, id.clone())?;
                    let id = PosixSpawnFileActionIdentifier::try_from(id).map_err(|_| {
                        vm.new_type_error("Unknown file_actions identifier".to_owned())
                    })?;
                    let args = FuncArgs::from(args.to_vec());
                    let ret = match id {
                        PosixSpawnFileActionIdentifier::Open => {
                            let (fd, path, oflag, mode): (_, PyPathLike, _, _) = args.bind(vm)?;
                            let path = ffi::CString::new(path.into_bytes()).map_err(|_| {
                                vm.new_value_error(
                                    "POSIX_SPAWN_OPEN path should not have nul bytes".to_owned(),
                                )
                            })?;
                            unsafe {
                                libc::posix_spawn_file_actions_addopen(
                                    &mut file_actions,
                                    fd,
                                    path.as_ptr(),
                                    oflag,
                                    mode,
                                )
                            }
                        }
                        PosixSpawnFileActionIdentifier::Close => {
                            let (fd,) = args.bind(vm)?;
                            unsafe {
                                libc::posix_spawn_file_actions_addclose(&mut file_actions, fd)
                            }
                        }
                        PosixSpawnFileActionIdentifier::Dup2 => {
                            let (fd, newfd) = args.bind(vm)?;
                            unsafe {
                                libc::posix_spawn_file_actions_adddup2(&mut file_actions, fd, newfd)
                            }
                        }
                    };
                    if ret != 0 {
                        return Err(errno_err(vm));
                    }
                }
            }

            let mut attrp = unsafe {
                let mut sa = std::mem::MaybeUninit::uninit();
                assert!(libc::posix_spawnattr_init(sa.as_mut_ptr()) == 0);
                sa.assume_init()
            };
            if let Some(sigs) = self.setsigdef {
                use nix::sys::signal;
                let mut set = signal::SigSet::empty();
                for sig in sigs.iter(vm)? {
                    let sig = sig?;
                    let sig = signal::Signal::try_from(sig).map_err(|_| {
                        vm.new_value_error(format!("signal number {} out of range", sig))
                    })?;
                    set.add(sig);
                }
                assert!(
                    unsafe { libc::posix_spawnattr_setsigdefault(&mut attrp, set.as_ref()) } == 0
                );
            }

            let mut args: Vec<ffi::CString> = self
                .args
                .iter(vm)?
                .map(|res| {
                    ffi::CString::new(res?.into_bytes()).map_err(|_| {
                        vm.new_value_error("path should not have nul bytes".to_owned())
                    })
                })
                .collect::<Result<_, _>>()?;
            let argv: Vec<*mut libc::c_char> = args
                .iter_mut()
                .map(|s| s.as_ptr() as _)
                .chain(std::iter::once(std::ptr::null_mut()))
                .collect();
            let mut env = envp_from_dict(self.env.into_dict(), vm)?;
            let envp: Vec<*mut libc::c_char> = env
                .iter_mut()
                .map(|s| s.as_ptr() as _)
                .chain(std::iter::once(std::ptr::null_mut()))
                .collect();

            let mut pid = 0;
            let ret = unsafe {
                if spawnp {
                    libc::posix_spawnp(
                        &mut pid,
                        path.as_ptr(),
                        &file_actions,
                        &attrp,
                        argv.as_ptr(),
                        envp.as_ptr(),
                    )
                } else {
                    libc::posix_spawn(
                        &mut pid,
                        path.as_ptr(),
                        &file_actions,
                        &attrp,
                        argv.as_ptr(),
                        envp.as_ptr(),
                    )
                }
            };

            if ret == 0 {
                Ok(pid)
            } else {
                Err(errno_err(vm))
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyfunction]
    fn posix_spawn(args: PosixSpawnArgs, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
        args.spawn(false, vm)
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyfunction]
    fn posix_spawnp(args: PosixSpawnArgs, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
        args.spawn(true, vm)
    }

    #[pyfunction(name = "WIFSIGNALED")]
    fn wifsignaled(status: i32) -> bool {
        libc::WIFSIGNALED(status)
    }
    #[pyfunction(name = "WIFSTOPPED")]
    fn wifstopped(status: i32) -> bool {
        libc::WIFSTOPPED(status)
    }
    #[pyfunction(name = "WIFEXITED")]
    fn wifexited(status: i32) -> bool {
        libc::WIFEXITED(status)
    }
    #[pyfunction(name = "WTERMSIG")]
    fn wtermsig(status: i32) -> i32 {
        libc::WTERMSIG(status)
    }
    #[pyfunction(name = "WSTOPSIG")]
    fn wstopsig(status: i32) -> i32 {
        libc::WSTOPSIG(status)
    }
    #[pyfunction(name = "WEXITSTATUS")]
    fn wexitstatus(status: i32) -> i32 {
        libc::WEXITSTATUS(status)
    }

    #[pyfunction]
    fn waitpid(pid: libc::pid_t, opt: i32, vm: &VirtualMachine) -> PyResult<(libc::pid_t, i32)> {
        let mut status = 0;
        let pid = unsafe { libc::waitpid(pid, &mut status, opt) };
        let pid = Errno::result(pid).map_err(|err| err.into_pyexception(vm))?;
        Ok((pid, status))
    }
    #[pyfunction]
    fn wait(vm: &VirtualMachine) -> PyResult<(libc::pid_t, i32)> {
        waitpid(-1, 0, vm)
    }

    #[pyfunction]
    fn kill(pid: i32, sig: isize, vm: &VirtualMachine) -> PyResult<()> {
        {
            let ret = unsafe { libc::kill(pid, sig as i32) };
            if ret == -1 {
                Err(errno_err(vm))
            } else {
                Ok(())
            }
        }
    }

    #[pyfunction]
    fn get_terminal_size(fd: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let (columns, lines) = {
            #[cfg(unix)]
            {
                nix::ioctl_read_bad!(winsz, libc::TIOCGWINSZ, libc::winsize);
                let mut w = libc::winsize {
                    ws_row: 0,
                    ws_col: 0,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                };
                unsafe { winsz(fd.unwrap_or(libc::STDOUT_FILENO), &mut w) }
                    .map_err(|err| err.into_pyexception(vm))?;
                (w.ws_col.into(), w.ws_row.into())
            }
        };
        super::_os::PyTerminalSize { columns, lines }.into_struct_sequence(vm)
    }

    // from libstd:
    // https://github.com/rust-lang/rust/blob/daecab3a784f28082df90cebb204998051f3557d/src/libstd/sys/unix/fs.rs#L1251
    #[cfg(target_os = "macos")]
    extern "C" {
        fn fcopyfile(
            in_fd: libc::c_int,
            out_fd: libc::c_int,
            state: *mut libc::c_void, // copyfile_state_t (unused)
            flags: u32,               // copyfile_flags_t
        ) -> libc::c_int;
    }

    #[cfg(target_os = "macos")]
    #[pyfunction]
    fn _fcopyfile(in_fd: i32, out_fd: i32, flags: i32, vm: &VirtualMachine) -> PyResult<()> {
        let ret = unsafe { fcopyfile(in_fd, out_fd, std::ptr::null_mut(), flags as u32) };
        if ret < 0 {
            Err(errno_err(vm))
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn dup(fd: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let fd = nix::unistd::dup(fd).map_err(|e| e.into_pyexception(vm))?;
        raw_set_inheritable(fd, false).map(|()| fd).map_err(|e| {
            let _ = nix::unistd::close(fd);
            e.into_pyexception(vm)
        })
    }

    pub(super) fn support_funcs(vm: &VirtualMachine) -> Vec<SupportFunc> {
        vec![
            SupportFunc::new(vm, "chmod", chmod, Some(false), Some(false), Some(false)),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new(vm, "chroot", chroot, Some(false), None, None),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new(vm, "chown", chown, Some(true), Some(true), Some(true)),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new(vm, "lchown", lchown, None, None, None),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new(vm, "fchown", fchown, Some(true), None, Some(true)),
            SupportFunc::new(vm, "umask", umask, Some(false), Some(false), Some(false)),
            SupportFunc::new(vm, "execv", execv, None, None, None),
        ]
    }

    /// Return a string containing the name of the user logged in on the
    /// controlling terminal of the process.
    ///
    /// Exceptions:
    ///
    /// - `OSError`: Raised if login name could not be determined (`getlogin()`
    ///   returned a null pointer).
    /// - `UnicodeDecodeError`: Raised if login name contained invalid UTF-8 bytes.
    #[pyfunction]
    fn getlogin(vm: &VirtualMachine) -> PyResult<String> {
        // Get a pointer to the login name string. The string is statically
        // allocated and might be overwritten on subsequent calls to this
        // function or to `cuserid()`. See man getlogin(3) for more information.
        let ptr = unsafe { libc::getlogin() };
        if ptr.is_null() {
            return Err(vm.new_os_error("unable to determine login name".to_owned()));
        }
        let slice = unsafe { ffi::CStr::from_ptr(ptr) };
        slice
            .to_str()
            .map(|s| s.to_owned())
            .map_err(|e| vm.new_unicode_decode_error(format!("unable to decode login name: {}", e)))
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn getgrouplist(user: PyStrRef, group: u32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let user = ffi::CString::new(user.borrow_value()).unwrap();
        let gid = Gid::from_raw(group);
        let group_ids = unistd::getgrouplist(&user, gid).map_err(|err| err.into_pyexception(vm))?;
        Ok(vm.ctx.new_list(
            group_ids
                .into_iter()
                .map(|gid| vm.ctx.new_int(gid.as_raw()))
                .collect(),
        ))
    }

    cfg_if::cfg_if! {
        if #[cfg(all(target_os = "linux", target_env = "gnu"))] {
            type PriorityWhichType = libc::__priority_which_t;
        } else {
            type PriorityWhichType = libc::c_int;
        }
    }
    cfg_if::cfg_if! {
        if #[cfg(target_os = "freebsd")] {
            type PriorityWhoType = i32;
        } else {
            type PriorityWhoType = u32;
        }
    }

    #[cfg(not(any(windows, target_os = "redox")))]
    #[pyfunction]
    fn getpriority(
        which: PriorityWhichType,
        who: PriorityWhoType,
        vm: &VirtualMachine,
    ) -> PyResult {
        Errno::clear();
        let retval = unsafe { libc::getpriority(which, who) };
        if errno() != 0 {
            Err(errno_err(vm))
        } else {
            Ok(vm.ctx.new_int(retval))
        }
    }

    #[cfg(not(any(windows, target_os = "redox")))]
    #[pyfunction]
    fn setpriority(
        which: PriorityWhichType,
        who: PriorityWhoType,
        priority: i32,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let retval = unsafe { libc::setpriority(which, who, priority) };
        if retval == -1 {
            Err(errno_err(vm))
        } else {
            Ok(())
        }
    }
}
#[cfg(unix)]
use posix as platform;
#[cfg(unix)]
pub(crate) use posix::raw_set_inheritable;

#[cfg(windows)]
#[pymodule]
mod nt {
    use super::*;
    use crate::builtins::list::PyListRef;
    pub(super) use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::RawHandle;
    #[cfg(target_env = "msvc")]
    use winapi::vc::vcruntime::intptr_t;

    #[pyattr]
    use libc::O_BINARY;

    pub(crate) type OpenFlags = u32;

    pub fn raw_file_number(handle: File) -> i64 {
        use std::os::windows::io::IntoRawHandle;

        handle.into_raw_handle() as i64
    }

    pub fn rust_file(raw_fileno: i64) -> File {
        use std::os::windows::io::{AsRawHandle, FromRawHandle};

        let raw_fileno = match raw_fileno {
            0 => io::stdin().as_raw_handle(),
            1 => io::stdout().as_raw_handle(),
            2 => io::stderr().as_raw_handle(),
            fno => fno as RawHandle,
        };

        //This seems to work as expected but further testing is required.
        unsafe { File::from_raw_handle(raw_fileno) }
    }

    impl PyPathLike {
        pub fn wide(&self) -> Vec<u16> {
            use std::os::windows::ffi::OsStrExt;
            self.path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }
    }

    #[pyfunction]
    pub(super) fn access(path: PyPathLike, mode: u8) -> bool {
        use winapi::um::{fileapi, winnt};
        let attr = unsafe { fileapi::GetFileAttributesW(path.wide().as_ptr()) };
        attr != fileapi::INVALID_FILE_ATTRIBUTES
            && (mode & 2 == 0
                || attr & winnt::FILE_ATTRIBUTE_READONLY == 0
                || attr & winnt::FILE_ATTRIBUTE_DIRECTORY != 0)
    }

    #[pyfunction]
    pub(super) fn symlink(
        src: PyPathLike,
        dst: PyPathLike,
        _target_is_directory: TargetIsDirectory,
        _dir_fd: DirFd,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let body = move || {
            use std::os::windows::fs as win_fs;
            let meta = fs::metadata(src.path.clone())?;
            if meta.is_file() {
                win_fs::symlink_file(src.path, dst.path)
            } else if meta.is_dir() {
                win_fs::symlink_dir(src.path, dst.path)
            } else {
                panic!("Unknown file type");
            }
        };
        body().map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn set_inheritable(fd: i64, inheritable: bool, vm: &VirtualMachine) -> PyResult<()> {
        #[cfg(windows)]
        {
            use winapi::um::{handleapi, winbase};
            let fd = fd as RawHandle;
            let flags = if inheritable {
                winbase::HANDLE_FLAG_INHERIT
            } else {
                0
            };
            let ret =
                unsafe { handleapi::SetHandleInformation(fd, winbase::HANDLE_FLAG_INHERIT, flags) };
            if ret == 0 {
                Err(errno_err(vm))
            } else {
                Ok(())
            }
        }
    }

    #[pyattr]
    fn environ(vm: &VirtualMachine) -> PyDictRef {
        let environ = vm.ctx.new_dict();

        for (key, value) in env::vars() {
            environ
                .set_item(vm.ctx.new_str(key), vm.ctx.new_str(value), vm)
                .unwrap();
        }
        environ
    }

    #[pyfunction]
    fn chmod(
        path: PyPathLike,
        dir_fd: DirFd,
        mode: u32,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        const S_IWRITE: u32 = 128;
        let path = make_path(vm, &path, &dir_fd)?;
        let metadata = if follow_symlinks.0 {
            fs::metadata(&path)
        } else {
            fs::symlink_metadata(&path)
        };
        let meta = metadata.map_err(|err| err.into_pyexception(vm))?;
        let mut permissions = meta.permissions();
        permissions.set_readonly(mode & S_IWRITE != 0);
        fs::set_permissions(&path, permissions).map_err(|err| err.into_pyexception(vm))
    }

    // cwait is available on MSVC only (according to CPython)
    #[cfg(target_env = "msvc")]
    extern "C" {
        fn _cwait(termstat: *mut i32, procHandle: intptr_t, action: i32) -> intptr_t;
        fn _get_errno(pValue: *mut i32) -> i32;
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn waitpid(pid: intptr_t, opt: i32, vm: &VirtualMachine) -> PyResult<(intptr_t, i32)> {
        let mut status = 0;
        let pid = unsafe { suppress_iph!(_cwait(&mut status, pid, opt)) };
        if pid == -1 {
            Err(errno_err(vm))
        } else {
            Ok((pid, status << 8))
        }
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn wait(vm: &VirtualMachine) -> PyResult<(intptr_t, i32)> {
        waitpid(-1, 0, vm)
    }

    #[pyfunction]
    fn kill(pid: i32, sig: isize, vm: &VirtualMachine) -> PyResult<()> {
        {
            use winapi::um::{handleapi, processthreadsapi, wincon, winnt};
            let sig = sig as u32;
            let pid = pid as u32;

            if sig == wincon::CTRL_C_EVENT || sig == wincon::CTRL_BREAK_EVENT {
                let ret = unsafe { wincon::GenerateConsoleCtrlEvent(sig, pid) };
                let res = if ret == 0 { Err(errno_err(vm)) } else { Ok(()) };
                return res;
            }

            let h = unsafe { processthreadsapi::OpenProcess(winnt::PROCESS_ALL_ACCESS, 0, pid) };
            if h.is_null() {
                return Err(errno_err(vm));
            }
            let ret = unsafe { processthreadsapi::TerminateProcess(h, sig) };
            let res = if ret == 0 { Err(errno_err(vm)) } else { Ok(()) };
            unsafe { handleapi::CloseHandle(h) };
            res
        }
    }

    #[pyfunction]
    fn get_terminal_size(fd: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let (columns, lines) = {
            {
                use winapi::um::{handleapi, processenv, winbase, wincon};
                let stdhandle = match fd {
                    OptionalArg::Present(0) => winbase::STD_INPUT_HANDLE,
                    OptionalArg::Present(1) | OptionalArg::Missing => winbase::STD_OUTPUT_HANDLE,
                    OptionalArg::Present(2) => winbase::STD_ERROR_HANDLE,
                    _ => return Err(vm.new_value_error("bad file descriptor".to_owned())),
                };
                let h = unsafe { processenv::GetStdHandle(stdhandle) };
                if h.is_null() {
                    return Err(vm.new_os_error("handle cannot be retrieved".to_owned()));
                }
                if h == handleapi::INVALID_HANDLE_VALUE {
                    return Err(errno_err(vm));
                }
                let mut csbi = wincon::CONSOLE_SCREEN_BUFFER_INFO::default();
                let ret = unsafe { wincon::GetConsoleScreenBufferInfo(h, &mut csbi) };
                if ret == 0 {
                    return Err(errno_err(vm));
                }
                let w = csbi.srWindow;
                (
                    (w.Right - w.Left + 1) as usize,
                    (w.Bottom - w.Top + 1) as usize,
                )
            }
        };
        super::_os::PyTerminalSize { columns, lines }.into_struct_sequence(vm)
    }

    #[cfg(target_env = "msvc")]
    type InvalidParamHandler = extern "C" fn(
        *const libc::wchar_t,
        *const libc::wchar_t,
        *const libc::wchar_t,
        libc::c_uint,
        libc::uintptr_t,
    );
    #[cfg(target_env = "msvc")]
    extern "C" {
        #[doc(hidden)]
        pub fn _set_thread_local_invalid_parameter_handler(
            pNew: InvalidParamHandler,
        ) -> InvalidParamHandler;
    }

    #[cfg(target_env = "msvc")]
    #[doc(hidden)]
    pub extern "C" fn silent_iph_handler(
        _: *const libc::wchar_t,
        _: *const libc::wchar_t,
        _: *const libc::wchar_t,
        _: libc::c_uint,
        _: libc::uintptr_t,
    ) {
    }

    #[cfg(target_env = "msvc")]
    extern "C" {
        fn _wexecv(cmdname: *const u16, argv: *const *const u16) -> intptr_t;
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn execv(
        path: PyStrRef,
        argv: Either<PyListRef, PyTupleRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        use std::iter::once;
        use std::os::windows::prelude::*;
        use std::str::FromStr;

        let path: Vec<u16> = ffi::OsString::from_str(path.borrow_value())
            .unwrap()
            .encode_wide()
            .chain(once(0u16))
            .collect();

        let argv: Vec<ffi::OsString> = vm.extract_elements(argv.as_object())?;

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("execv() arg 2 must not be empty".to_owned()))?;

        if first.is_empty() {
            return Err(
                vm.new_value_error("execv() arg 2 first element cannot be empty".to_owned())
            );
        }

        let argv: Vec<Vec<u16>> = argv
            .into_iter()
            .map(|s| s.encode_wide().chain(once(0u16)).collect())
            .collect();

        let argv_execv: Vec<*const u16> = argv
            .iter()
            .map(|v| v.as_ptr())
            .chain(once(std::ptr::null()))
            .collect();

        if (unsafe { suppress_iph!(_wexecv(path.as_ptr(), argv_execv.as_ptr())) } == -1) {
            Err(errno_err(vm))
        } else {
            Ok(())
        }
    }

    pub(super) fn support_funcs(_vm: &VirtualMachine) -> Vec<SupportFunc> {
        Vec::new()
    }
}
#[cfg(windows)]
use nt as platform;
#[cfg(windows)]
pub use nt::{_set_thread_local_invalid_parameter_handler, silent_iph_handler};

#[cfg(not(any(unix, windows)))]
#[pymodule(name = "posix")]
mod minor {
    use super::*;

    #[cfg(target_os = "wasi")]
    pub(crate) type OpenFlags = u16;

    #[cfg(target_os = "wasi")]
    pub(crate) fn raw_file_number(handle: File) -> i64 {
        // This should be safe, since the wasi api is pretty well defined, but once
        // `wasi_ext` gets stabilized we should use that instead.
        unsafe { std::mem::transmute::<_, u32>(handle).into() }
    }
    #[cfg(not(target_os = "wasi"))]
    pub(crate) fn raw_file_number(_handle: File) -> i64 {
        unimplemented!();
    }

    #[cfg(target_os = "wasi")]
    pub(crate) fn rust_file(raw_fileno: i64) -> File {
        unsafe { std::mem::transmute(raw_fileno as u32) }
    }

    #[cfg(not(target_os = "wasi"))]
    pub(crate) fn rust_file(_raw_fileno: i64) -> File {
        unimplemented!();
    }

    #[pyfunction]
    pub(super) fn access(_path: PyStrRef, _mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        os_unimpl("os.access", vm)
    }

    #[pyfunction]
    pub(super) fn symlink(
        _src: PyPathLike,
        _dst: PyPathLike,
        _target_is_directory: TargetIsDirectory,
        _dir_fd: DirFd,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        os_unimpl("os.symlink", vm)
    }

    #[pyattr]
    fn environ(vm: &VirtualMachine) -> PyDictRef {
        vm.ctx.new_dict()
    }

    pub(super) fn support_funcs(_vm: &VirtualMachine) -> Vec<SupportFunc> {
        Vec::new()
    }
}
#[cfg(not(any(unix, windows)))]
use minor as platform;

pub(crate) use platform::{raw_file_number, rust_file, OpenFlags, MODULE_NAME};
