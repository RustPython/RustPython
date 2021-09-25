use super::errno::errors;
use crate::crt_fd::Fd;
use crate::{
    builtins::{PyBytes, PyBytesRef, PyInt, PySet, PyStr, PyStrRef},
    exceptions::{IntoPyException, PyBaseExceptionRef},
    function::{ArgumentError, FromArgs, FuncArgs},
    protocol::PyBuffer,
    IntoPyObject, PyObjectRef, PyResult, PyValue, TryFromBorrowedObject, TryFromObject,
    TypeProtocol, VirtualMachine,
};
use std::ffi;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::ffi as ffi_ext;
#[cfg(target_os = "wasi")]
use std::os::wasi::ffi as ffi_ext;

#[derive(Debug, Copy, Clone)]
pub(super) enum OutputMode {
    String,
    Bytes,
}

impl OutputMode {
    pub(super) fn process_path(self, path: impl Into<PathBuf>, vm: &VirtualMachine) -> PyResult {
        fn inner(mode: OutputMode, path: PathBuf, vm: &VirtualMachine) -> PyResult {
            let path_as_string = |p: PathBuf| {
                p.into_os_string().into_string().map_err(|_| {
                    vm.new_unicode_decode_error(
                        "Can't convert OS path to valid UTF-8 string".into(),
                    )
                })
            };
            match mode {
                OutputMode::String => path_as_string(path).map(|s| vm.ctx.new_utf8_str(s)),
                OutputMode::Bytes => {
                    #[cfg(any(unix, target_os = "wasi"))]
                    {
                        use ffi_ext::OsStringExt;
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

pub struct PyPathLike {
    pub path: PathBuf,
    pub(super) mode: OutputMode,
}

impl PyPathLike {
    pub fn new_str(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mode: OutputMode::String,
        }
    }

    #[cfg(any(unix, target_os = "wasi"))]
    pub fn into_bytes(self) -> Vec<u8> {
        use ffi_ext::OsStringExt;
        self.path.into_os_string().into_vec()
    }

    #[cfg(any(unix, target_os = "wasi"))]
    pub fn into_cstring(self, vm: &VirtualMachine) -> PyResult<ffi::CString> {
        ffi::CString::new(self.into_bytes()).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(windows)]
    pub fn to_widecstring(&self, vm: &VirtualMachine) -> PyResult<widestring::WideCString> {
        widestring::WideCString::from_os_str(&self.path).map_err(|err| err.into_pyexception(vm))
    }
}

pub(super) fn fs_metadata<P: AsRef<Path>>(
    path: P,
    follow_symlink: bool,
) -> io::Result<fs::Metadata> {
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

pub enum FsPath {
    Str(PyStrRef),
    Bytes(PyBytesRef),
}

impl FsPath {
    pub fn as_os_str(&self, vm: &VirtualMachine) -> PyResult<&ffi::OsStr> {
        // TODO: FS encodings
        match self {
            FsPath::Str(s) => Ok(s.as_str().as_ref()),
            FsPath::Bytes(b) => bytes_as_osstr(b.as_bytes(), vm),
        }
    }

    fn to_pathlike(&self, vm: &VirtualMachine) -> PyResult<PyPathLike> {
        let path = self.as_os_str(vm)?.to_owned().into();
        let mode = match self {
            Self::Str(_) => OutputMode::String,
            Self::Bytes(_) => OutputMode::Bytes,
        };
        Ok(PyPathLike { path, mode })
    }

    #[cfg(not(target_os = "redox"))]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        // TODO: FS encodings
        match self {
            FsPath::Str(s) => s.as_str().as_bytes(),
            FsPath::Bytes(b) => b.as_bytes(),
        }
    }
}

impl IntoPyObject for FsPath {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::Str(s) => s.into_object(),
            Self::Bytes(b) => b.into_object(),
        }
    }
}

pub(crate) fn fspath(
    obj: PyObjectRef,
    check_for_nul: bool,
    vm: &VirtualMachine,
) -> PyResult<FsPath> {
    // PyOS_FSPath in CPython
    let check_nul = |b: &[u8]| {
        if !check_for_nul || memchr::memchr(b'\0', b).is_none() {
            Ok(())
        } else {
            Err(crate::exceptions::cstring_error(vm))
        }
    };
    let match1 = |obj: PyObjectRef| {
        let pathlike = match_class!(match obj {
            s @ PyStr => {
                check_nul(s.as_str().as_bytes())?;
                FsPath::Str(s)
            }
            b @ PyBytes => {
                check_nul(&b)?;
                FsPath::Bytes(b)
            }
            obj => return Ok(Err(obj)),
        });
        Ok(Ok(pathlike))
    };
    let obj = match match1(obj)? {
        Ok(pathlike) => return Ok(pathlike),
        Err(obj) => obj,
    };
    let method = vm.get_method_or_type_error(obj.clone(), "__fspath__", || {
        format!(
            "expected str, bytes or os.PathLike object, not {}",
            obj.class().name()
        )
    })?;
    let result = vm.invoke(&method, ())?;
    match1(result)?.map_err(|result| {
        vm.new_type_error(format!(
            "expected {}.__fspath__() to return str or bytes, not {}",
            obj.class().name(),
            result.class().name(),
        ))
    })
}

impl TryFromObject for PyPathLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        // path_converter in CPython
        let obj = match PyBuffer::try_from_borrowed_object(vm, &obj) {
            Ok(buffer) => PyBytes::from(buffer.internal.obj_bytes().to_vec()).into_pyobject(vm),
            Err(_) => obj,
        };
        let fs_path = fspath(obj, true, vm)?;
        fs_path.to_pathlike(vm)
    }
}

pub(crate) enum PathOrFd {
    Path(PyPathLike),
    Fd(i32),
}

impl TryFromObject for PathOrFd {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match obj.downcast::<PyInt>() {
            Ok(int) => int.try_to_primitive(vm).map(Self::Fd),
            Err(obj) => PyPathLike::try_from_object(vm, obj).map(Self::Path),
        }
    }
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
        let msg = vm.ctx.new_utf8_str(self.to_string());
        vm.new_exception(exc_type, vec![errno, msg])
    }
}

#[cfg(unix)]
impl IntoPyException for nix::Error {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        io::Error::from(self).into_pyexception(vm)
    }
}

/// Convert the error stored in the `errno` variable into an Exception
#[inline]
pub fn errno_err(vm: &VirtualMachine) -> PyBaseExceptionRef {
    errno().into_pyexception(vm)
}

#[cfg(windows)]
pub fn errno() -> io::Error {
    let err = io::Error::last_os_error();
    // FIXME: probably not ideal, we need a bigger dichotomy between GetLastError and errno
    if err.raw_os_error() == Some(0) {
        io::Error::from_raw_os_error(super::msvcrt::get_errno())
    } else {
        err
    }
}

#[cfg(not(windows))]
pub fn errno() -> io::Error {
    io::Error::last_os_error()
}

#[allow(dead_code)]
#[derive(FromArgs, Default)]
pub struct TargetIsDirectory {
    #[pyarg(any, default = "false")]
    pub(crate) target_is_directory: bool,
}

cfg_if::cfg_if! {
    if #[cfg(all(any(unix, target_os = "wasi"), not(target_os = "redox")))] {
        use libc::AT_FDCWD;
    } else {
        const AT_FDCWD: i32 = -100;
    }
}
const DEFAULT_DIR_FD: Fd = Fd(AT_FDCWD);

// XXX: AVAILABLE should be a bool, but we can't yet have it as a bool and just cast it to usize
#[derive(Copy, Clone)]
pub struct DirFd<const AVAILABLE: usize>(pub(crate) [Fd; AVAILABLE]);

impl<const AVAILABLE: usize> Default for DirFd<AVAILABLE> {
    fn default() -> Self {
        Self([DEFAULT_DIR_FD; AVAILABLE])
    }
}

// not used on all platforms
#[allow(unused)]
impl DirFd<1> {
    #[inline(always)]
    pub(crate) fn fd_opt(&self) -> Option<Fd> {
        self.get_opt().map(Fd)
    }

    #[inline]
    pub(crate) fn get_opt(&self) -> Option<i32> {
        let fd = self.fd();
        if fd == DEFAULT_DIR_FD {
            None
        } else {
            Some(fd.0)
        }
    }

    #[inline(always)]
    pub(crate) fn fd(&self) -> Fd {
        self.0[0]
    }
}

impl<const AVAILABLE: usize> FromArgs for DirFd<AVAILABLE> {
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let fd = match args.take_keyword("dir_fd") {
            Some(o) if vm.is_none(&o) => DEFAULT_DIR_FD,
            None => DEFAULT_DIR_FD,
            Some(o) => {
                let fd = vm.to_index_opt(o.clone()).unwrap_or_else(|| {
                    Err(vm.new_type_error(format!(
                        "argument should be integer or None, not {}",
                        o.class().name()
                    )))
                })?;
                let fd = fd.try_to_primitive(vm)?;
                Fd(fd)
            }
        };
        if AVAILABLE == 0 && fd != DEFAULT_DIR_FD {
            return Err(vm
                .new_not_implemented_error("dir_fd unavailable on this platform".to_owned())
                .into());
        }
        Ok(Self([fd; AVAILABLE]))
    }
}

#[derive(FromArgs)]
pub(super) struct FollowSymlinks(
    #[pyarg(named, name = "follow_symlinks", default = "true")] pub bool,
);

#[cfg(unix)]
use platform::bytes_as_osstr;

#[cfg(not(unix))]
fn bytes_as_osstr<'a>(b: &'a [u8], vm: &VirtualMachine) -> PyResult<&'a ffi::OsStr> {
    std::str::from_utf8(b)
        .map(|s| s.as_ref())
        .map_err(|_| vm.new_unicode_decode_error("can't decode path for utf-8".to_owned()))
}

#[pymodule(name = "_os")]
pub(super) mod _os {
    use super::{
        errno_err, DirFd, FollowSymlinks, FsPath, OutputMode, PathOrFd, PyPathLike, SupportFunc,
    };
    use crate::common::lock::{OnceCell, PyRwLock};
    use crate::{
        builtins::{PyBytesRef, PyStrRef, PyTuple, PyTupleRef, PyTypeRef},
        byteslike::ArgBytesLike,
        crt_fd::{Fd, Offset},
        exceptions::IntoPyException,
        function::{FuncArgs, OptionalArg},
        slots::{IteratorIterable, PyIter},
        suppress_iph,
        utils::Either,
        vm::{ReprGuard, VirtualMachine},
        IntoPyObject, PyObjectRef, PyRef, PyResult, PyStructSequence, PyValue,
        TryFromBorrowedObject, TryFromObject, TypeProtocol,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use itertools::Itertools;
    use num_bigint::BigInt;
    use std::ffi;
    use std::fs::OpenOptions;
    use std::io::{self, Read, Write};
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};
    use std::{env, fs};

    const OPEN_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    pub(crate) const MKDIR_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    const STAT_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    const UTIME_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    pub(crate) const SYMLINK_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));

    #[pyattr]
    use libc::{
        O_APPEND, O_CREAT, O_EXCL, O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY, SEEK_CUR, SEEK_END,
        SEEK_SET,
    };

    #[pyattr]
    pub(crate) const F_OK: u8 = 0;
    #[pyattr]
    pub(crate) const R_OK: u8 = 1 << 2;
    #[pyattr]
    pub(crate) const W_OK: u8 = 1 << 1;
    #[pyattr]
    pub(crate) const X_OK: u8 = 1 << 0;

    #[pyfunction]
    fn close(fileno: i32, vm: &VirtualMachine) -> PyResult<()> {
        Fd(fileno).close().map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn closerange(fd_low: i32, fd_high: i32) {
        for fileno in fd_low..fd_high {
            let _ = Fd(fileno).close();
        }
    }

    #[cfg(any(unix, windows, target_os = "wasi"))]
    #[derive(FromArgs)]
    struct OpenArgs {
        #[pyarg(any)]
        path: PyPathLike,
        #[pyarg(any)]
        flags: i32,
        #[pyarg(any, default)]
        mode: Option<i32>,
        #[pyarg(flatten)]
        dir_fd: DirFd<{ OPEN_DIR_FD as usize }>,
    }

    #[pyfunction]
    fn open(args: OpenArgs, vm: &VirtualMachine) -> PyResult<i32> {
        os_open(args.path, args.flags, args.mode, args.dir_fd, vm)
    }

    #[cfg(any(unix, windows, target_os = "wasi"))]
    pub(crate) fn os_open(
        name: PyPathLike,
        flags: i32,
        mode: Option<i32>,
        dir_fd: DirFd<{ OPEN_DIR_FD as usize }>,
        vm: &VirtualMachine,
    ) -> PyResult<i32> {
        let mode = mode.unwrap_or(0o777);
        #[cfg(windows)]
        let fd = {
            let [] = dir_fd.0;
            let name = name.to_widecstring(vm)?;
            let flags = flags | libc::O_NOINHERIT;
            Fd::wopen(&name, flags, mode)
        };
        #[cfg(not(windows))]
        let fd = {
            let name = name.into_cstring(vm)?;
            #[cfg(not(target_os = "wasi"))]
            let flags = flags | libc::O_CLOEXEC;
            #[cfg(not(target_os = "redox"))]
            if let Some(dir_fd) = dir_fd.fd_opt() {
                dir_fd.openat(&name, flags, mode)
            } else {
                Fd::open(&name, flags, mode)
            }
            #[cfg(target_os = "redox")]
            {
                let [] = dir_fd.0;
                Fd::open(&name, flags, mode)
            }
        };
        fd.map(|fd| fd.0).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn fsync(fd: i32, vm: &VirtualMachine) -> PyResult<()> {
        Fd(fd).fsync().map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn read(fd: i32, n: usize, vm: &VirtualMachine) -> PyResult {
        let mut buffer = vec![0u8; n];
        let mut file = Fd(fd);
        let n = file
            .read(&mut buffer)
            .map_err(|err| err.into_pyexception(vm))?;
        buffer.truncate(n);

        Ok(vm.ctx.new_bytes(buffer))
    }

    #[pyfunction]
    fn write(fd: i32, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult {
        let mut file = Fd(fd);
        let written = data
            .with_ref(|b| file.write(b))
            .map_err(|err| err.into_pyexception(vm))?;

        Ok(vm.ctx.new_int(written))
    }

    #[pyfunction]
    #[pyfunction(name = "unlink")]
    fn remove(path: PyPathLike, dir_fd: DirFd<0>, vm: &VirtualMachine) -> PyResult<()> {
        let [] = dir_fd.0;
        let is_junction = cfg!(windows)
            && fs::metadata(&path).map_or(false, |meta| meta.file_type().is_dir())
            && fs::symlink_metadata(&path).map_or(false, |meta| meta.file_type().is_symlink());
        let res = if is_junction {
            fs::remove_dir(&path)
        } else {
            fs::remove_file(&path)
        };
        res.map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(windows))]
    #[pyfunction]
    fn mkdir(
        path: PyPathLike,
        mode: OptionalArg<i32>,
        dir_fd: DirFd<{ MKDIR_DIR_FD as usize }>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mode = mode.unwrap_or(0o777);
        let path = path.into_cstring(vm)?;
        #[cfg(not(target_os = "redox"))]
        if let Some(fd) = dir_fd.get_opt() {
            let res = unsafe { libc::mkdirat(fd, path.as_ptr(), mode as _) };
            let res = if res < 0 { Err(errno_err(vm)) } else { Ok(()) };
            return res;
        }
        #[cfg(target_os = "redox")]
        let [] = dir_fd.0;
        let res = unsafe { libc::mkdir(path.as_ptr(), mode as _) };
        if res < 0 {
            Err(errno_err(vm))
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn mkdirs(path: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        fs::create_dir_all(path.as_str()).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn rmdir(path: PyPathLike, dir_fd: DirFd<0>, vm: &VirtualMachine) -> PyResult<()> {
        let [] = dir_fd.0;
        fs::remove_dir(path).map_err(|err| err.into_pyexception(vm))
    }

    const LISTDIR_FD: bool = cfg!(all(unix, not(target_os = "redox")));

    #[pyfunction]
    fn listdir(path: OptionalArg<PathOrFd>, vm: &VirtualMachine) -> PyResult {
        let path = path.unwrap_or_else(|| PathOrFd::Path(PyPathLike::new_str(".")));
        let list = match path {
            PathOrFd::Path(path) => {
                let dir_iter = fs::read_dir(&path).map_err(|err| err.into_pyexception(vm))?;
                dir_iter
                    .map(|entry| match entry {
                        Ok(entry_path) => path.mode.process_path(entry_path.file_name(), vm),
                        Err(err) => Err(err.into_pyexception(vm)),
                    })
                    .collect::<PyResult<_>>()?
            }
            PathOrFd::Fd(fno) => {
                #[cfg(not(all(unix, not(target_os = "redox"))))]
                {
                    let _ = fno;
                    return Err(vm.new_not_implemented_error(
                        "can't pass fd to listdir on this platform".to_owned(),
                    ));
                }
                #[cfg(all(unix, not(target_os = "redox")))]
                {
                    use super::ffi_ext::OsStrExt;
                    let new_fd = nix::unistd::dup(fno).map_err(|e| e.into_pyexception(vm))?;
                    let mut dir =
                        nix::dir::Dir::from_fd(new_fd).map_err(|e| e.into_pyexception(vm))?;
                    dir.iter()
                        .filter_map(|entry| {
                            entry
                                .map_err(|e| e.into_pyexception(vm))
                                .and_then(|entry| {
                                    let fname = entry.file_name().to_bytes();
                                    Ok(match fname {
                                        b"." | b".." => None,
                                        _ => Some(
                                            OutputMode::String
                                                .process_path(ffi::OsStr::from_bytes(fname), vm)?,
                                        ),
                                    })
                                })
                                .transpose()
                        })
                        .collect::<PyResult<_>>()?
                }
            }
        };
        Ok(vm.ctx.new_list(list))
    }

    #[pyfunction]
    fn putenv(
        key: Either<PyStrRef, PyBytesRef>,
        value: Either<PyStrRef, PyBytesRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let key: &ffi::OsStr = match key {
            Either::A(ref s) => s.as_str().as_ref(),
            Either::B(ref b) => super::bytes_as_osstr(b.as_bytes(), vm)?,
        };
        let value: &ffi::OsStr = match value {
            Either::A(ref s) => s.as_str().as_ref(),
            Either::B(ref b) => super::bytes_as_osstr(b.as_bytes(), vm)?,
        };
        env::set_var(key, value);
        Ok(())
    }

    #[pyfunction]
    fn unsetenv(key: Either<PyStrRef, PyBytesRef>, vm: &VirtualMachine) -> PyResult<()> {
        let key: &ffi::OsStr = match key {
            Either::A(ref s) => s.as_str().as_ref(),
            Either::B(ref b) => super::bytes_as_osstr(b.as_bytes(), vm)?,
        };
        env::remove_var(key);
        Ok(())
    }

    #[pyfunction]
    fn readlink(path: PyPathLike, dir_fd: DirFd<0>, vm: &VirtualMachine) -> PyResult {
        let mode = path.mode;
        let [] = dir_fd.0;
        let path = fs::read_link(path).map_err(|err| err.into_pyexception(vm))?;
        mode.process_path(path, vm)
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyValue)]
    struct DirEntry {
        entry: fs::DirEntry,
        mode: OutputMode,
        stat: OnceCell<PyObjectRef>,
        lstat: OnceCell<PyObjectRef>,
        #[cfg(not(unix))]
        ino: AtomicCell<Option<u64>>,
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
            match super::fs_metadata(self.entry.path(), follow_symlinks.0) {
                Ok(meta) => Ok(action(meta)),
                Err(e) => {
                    // FileNotFoundError is caught and not raised
                    if e.kind() == io::ErrorKind::NotFound {
                        Ok(false)
                    } else {
                        Err(e.into_pyexception(vm))
                    }
                }
            }
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
            dir_fd: DirFd<{ STAT_DIR_FD as usize }>,
            follow_symlinks: FollowSymlinks,
            vm: &VirtualMachine,
        ) -> PyResult {
            let do_stat = |follow_symlinks| {
                stat(
                    PathOrFd::Path(PyPathLike {
                        path: self.entry.path(),
                        mode: OutputMode::String,
                    }),
                    dir_fd,
                    FollowSymlinks(follow_symlinks),
                    vm,
                )
            };
            let lstat = || self.lstat.get_or_try_init(|| do_stat(false));
            let stat = if follow_symlinks.0 {
                // if follow_symlinks == true and we aren't a symlink, cache both stat and lstat
                self.stat.get_or_try_init(|| {
                    if self.is_symlink(vm)? {
                        do_stat(true)
                    } else {
                        lstat().map(Clone::clone)
                    }
                })?
            } else {
                lstat()?
            };
            Ok(stat.clone())
        }

        #[cfg(not(unix))]
        #[pymethod]
        fn inode(&self, vm: &VirtualMachine) -> PyResult<u64> {
            match self.ino.load() {
                Some(ino) => Ok(ino),
                None => {
                    let stat = stat_inner(
                        PathOrFd::Path(PyPathLike {
                            path: self.entry.path(),
                            mode: OutputMode::String,
                        }),
                        DirFd::default(),
                        FollowSymlinks(false),
                    )
                    .map_err(|e| e.into_pyexception(vm))?
                    .ok_or_else(|| crate::exceptions::cstring_error(vm))?;
                    // Err(T) means other thread set `ino` at the mean time which is safe to ignore
                    let _ = self.ino.compare_exchange(None, Some(stat.st_ino));
                    Ok(stat.st_ino)
                }
            }
        }

        #[cfg(unix)]
        #[pymethod]
        fn inode(&self, _vm: &VirtualMachine) -> PyResult<u64> {
            use std::os::unix::fs::DirEntryExt;
            Ok(self.entry.ino())
        }

        #[pymethod(magic)]
        fn fspath(&self, vm: &VirtualMachine) -> PyResult {
            self.path(vm)
        }

        #[pymethod(magic)]
        fn repr(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            let name = match vm.get_attribute(zelf.clone(), "name") {
                Ok(name) => Some(name),
                Err(e)
                    if e.isinstance(&vm.ctx.exceptions.attribute_error)
                        || e.isinstance(&vm.ctx.exceptions.value_error) =>
                {
                    None
                }
                Err(e) => return Err(e),
            };
            if let Some(name) = name {
                if let Some(_guard) = ReprGuard::enter(vm, &zelf) {
                    let repr = vm.to_repr(&name)?;
                    Ok(format!("<{} {}>", zelf.class(), repr))
                } else {
                    Err(vm.new_runtime_error(format!(
                        "reentrant call inside {}.__repr__",
                        zelf.class()
                    )))
                }
            } else {
                Ok(format!("<{}>", zelf.class()))
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "ScandirIter")]
    #[derive(Debug, PyValue)]
    struct ScandirIterator {
        entries: PyRwLock<fs::ReadDir>,
        exhausted: AtomicCell<bool>,
        mode: OutputMode,
    }

    #[pyimpl(with(PyIter))]
    impl ScandirIterator {
        #[pymethod]
        fn close(&self) {
            self.exhausted.store(true);
        }

        #[pymethod(magic)]
        fn enter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod(magic)]
        fn exit(zelf: PyRef<Self>, _args: FuncArgs) {
            zelf.close()
        }
    }
    impl IteratorIterable for ScandirIterator {}
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
                        lstat: OnceCell::new(),
                        stat: OnceCell::new(),
                        #[cfg(not(unix))]
                        ino: AtomicCell::new(None),
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
    #[derive(Debug, PyStructSequence, FromArgs)]
    struct StatResult {
        #[pyarg(any)]
        pub st_mode: BigInt,
        #[pyarg(any)]
        pub st_ino: BigInt,
        #[pyarg(any)]
        pub st_dev: BigInt,
        #[pyarg(any)]
        pub st_nlink: BigInt,
        #[pyarg(any)]
        pub st_uid: BigInt,
        #[pyarg(any)]
        pub st_gid: BigInt,
        #[pyarg(any)]
        pub st_size: BigInt,
        // TODO: unnamed structsequence fields
        #[pyarg(positional, default)]
        pub __st_atime_int: BigInt,
        #[pyarg(positional, default)]
        pub __st_mtime_int: BigInt,
        #[pyarg(positional, default)]
        pub __st_ctime_int: BigInt,
        #[pyarg(any, default)]
        pub st_atime: f64,
        #[pyarg(any, default)]
        pub st_mtime: f64,
        #[pyarg(any, default)]
        pub st_ctime: f64,
        #[pyarg(any, default)]
        pub st_atime_ns: BigInt,
        #[pyarg(any, default)]
        pub st_mtime_ns: BigInt,
        #[pyarg(any, default)]
        pub st_ctime_ns: BigInt,
    }

    #[pyimpl(with(PyStructSequence))]
    impl StatResult {
        fn from_stat(stat: &StatStruct) -> Self {
            let (atime, mtime, ctime);
            #[cfg(any(unix, windows))]
            {
                atime = (stat.st_atime, stat.st_atime_nsec);
                mtime = (stat.st_mtime, stat.st_mtime_nsec);
                ctime = (stat.st_ctime, stat.st_ctime_nsec);
            }
            #[cfg(target_os = "wasi")]
            {
                atime = (stat.st_atim.tv_sec, stat.st_atim.tv_nsec);
                mtime = (stat.st_mtim.tv_sec, stat.st_mtim.tv_nsec);
                ctime = (stat.st_ctim.tv_sec, stat.st_ctim.tv_nsec);
            }

            const NANOS_PER_SEC: u32 = 1_000_000_000;
            let to_f64 = |(s, ns)| (s as f64) + (ns as f64) / (NANOS_PER_SEC as f64);
            let to_ns = |(s, ns)| s as i128 * NANOS_PER_SEC as i128 + ns as i128;
            StatResult {
                st_mode: stat.st_mode.into(),
                st_ino: stat.st_ino.into(),
                st_dev: stat.st_dev.into(),
                st_nlink: stat.st_nlink.into(),
                st_uid: stat.st_uid.into(),
                st_gid: stat.st_gid.into(),
                st_size: stat.st_size.into(),
                __st_atime_int: atime.0.into(),
                __st_mtime_int: mtime.0.into(),
                __st_ctime_int: ctime.0.into(),
                st_atime: to_f64(atime),
                st_mtime: to_f64(mtime),
                st_ctime: to_f64(ctime),
                st_atime_ns: to_ns(atime).into(),
                st_mtime_ns: to_ns(mtime).into(),
                st_ctime_ns: to_ns(ctime).into(),
            }
        }

        #[pyslot]
        fn tp_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let flatten_args = |r: &[PyObjectRef]| {
                let mut vec_args = Vec::from(r);
                loop {
                    if let Ok(obj) = vec_args.iter().exactly_one() {
                        match obj.payload::<PyTuple>() {
                            Some(t) => {
                                vec_args = Vec::from(t.as_slice());
                            }
                            None => {
                                return vec_args;
                            }
                        }
                    } else {
                        return vec_args;
                    }
                }
            };

            let args: FuncArgs = flatten_args(args.args.as_slice()).into();

            let stat: StatResult = args.bind(vm)?;
            Ok(stat.into_pyobject(vm))
        }
    }

    #[cfg(not(windows))]
    use libc::stat as StatStruct;

    #[cfg(windows)]
    struct StatStruct {
        st_dev: libc::c_ulong,
        st_ino: u64,
        st_mode: libc::c_ushort,
        st_nlink: i32,
        st_uid: i32,
        st_gid: i32,
        st_size: u64,
        st_atime: libc::time_t,
        st_atime_nsec: i32,
        st_mtime: libc::time_t,
        st_mtime_nsec: i32,
        st_ctime: libc::time_t,
        st_ctime_nsec: i32,
    }

    #[cfg(windows)]
    fn meta_to_stat(meta: &fs::Metadata) -> io::Result<StatStruct> {
        let st_mode = {
            // Based on CPython fileutils.c' attributes_to_mode
            let mut m = 0;
            if meta.is_dir() {
                m |= libc::S_IFDIR | 0o111; /* IFEXEC for user,group,other */
            } else {
                m |= libc::S_IFREG;
            }
            if meta.permissions().readonly() {
                m |= 0o444;
            } else {
                m |= 0o666;
            }
            m as _
        };
        let (atime, mtime, ctime) = (meta.accessed()?, meta.modified()?, meta.created()?);
        let sec = |systime: SystemTime| match systime.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => d.as_secs() as libc::time_t,
            Err(e) => -(e.duration().as_secs() as libc::time_t),
        };
        let nsec = |systime: SystemTime| match systime.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => d.subsec_nanos() as i32,
            Err(e) => -(e.duration().subsec_nanos() as i32),
        };
        Ok(StatStruct {
            st_dev: 0,
            st_ino: 0,
            st_mode,
            st_nlink: 0,
            st_uid: 0,
            st_gid: 0,
            st_size: meta.len(),
            st_atime: sec(atime),
            st_mtime: sec(mtime),
            st_ctime: sec(ctime),
            st_atime_nsec: nsec(atime),
            st_mtime_nsec: nsec(mtime),
            st_ctime_nsec: nsec(ctime),
        })
    }

    #[cfg(windows)]
    fn stat_inner(
        file: PathOrFd,
        dir_fd: DirFd<{ STAT_DIR_FD as usize }>,
        follow_symlinks: FollowSymlinks,
    ) -> io::Result<Option<StatStruct>> {
        // TODO: replicate CPython's win32_xstat
        let [] = dir_fd.0;
        let meta = match file {
            PathOrFd::Path(path) => super::fs_metadata(&path, follow_symlinks.0)?,
            PathOrFd::Fd(fno) => Fd(fno).as_rust_file()?.metadata()?,
        };
        meta_to_stat(&meta).map(Some)
    }

    #[cfg(not(windows))]
    fn stat_inner(
        file: PathOrFd,
        dir_fd: DirFd<{ STAT_DIR_FD as usize }>,
        follow_symlinks: FollowSymlinks,
    ) -> io::Result<Option<StatStruct>> {
        let mut stat = std::mem::MaybeUninit::uninit();
        let ret = match file {
            PathOrFd::Path(path) => {
                use super::ffi_ext::OsStrExt;
                let path = path.as_ref().as_os_str().as_bytes();
                let path = match ffi::CString::new(path) {
                    Ok(x) => x,
                    Err(_) => return Ok(None),
                };

                #[cfg(not(target_os = "redox"))]
                let fstatat_ret = dir_fd.get_opt().map(|dir_fd| {
                    let flags = if follow_symlinks.0 {
                        0
                    } else {
                        libc::AT_SYMLINK_NOFOLLOW
                    };
                    unsafe { libc::fstatat(dir_fd, path.as_ptr(), stat.as_mut_ptr(), flags) }
                });
                #[cfg(target_os = "redox")]
                let ([], fstatat_ret) = (dir_fd.0, None);

                fstatat_ret.unwrap_or_else(|| {
                    if follow_symlinks.0 {
                        unsafe { libc::stat(path.as_ptr(), stat.as_mut_ptr()) }
                    } else {
                        unsafe { libc::lstat(path.as_ptr(), stat.as_mut_ptr()) }
                    }
                })
            }
            PathOrFd::Fd(fd) => unsafe { libc::fstat(fd, stat.as_mut_ptr()) },
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(unsafe { stat.assume_init() }))
    }

    #[pyfunction]
    #[pyfunction(name = "fstat")]
    fn stat(
        file: PathOrFd,
        dir_fd: DirFd<{ STAT_DIR_FD as usize }>,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult {
        let stat = stat_inner(file, dir_fd, follow_symlinks)
            .map_err(|e| e.into_pyexception(vm))?
            .ok_or_else(|| crate::exceptions::cstring_error(vm))?;
        Ok(StatResult::from_stat(&stat).into_pyobject(vm))
    }

    #[pyfunction]
    fn lstat(
        file: PathOrFd,
        dir_fd: DirFd<{ STAT_DIR_FD as usize }>,
        vm: &VirtualMachine,
    ) -> PyResult {
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
    fn fspath(path: PyObjectRef, vm: &VirtualMachine) -> PyResult<FsPath> {
        super::fspath(path, false, vm)
    }

    #[pyfunction]
    #[pyfunction(name = "replace")]
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
            use winapi::um::{fileapi, winnt};
            let handle = Fd(fd).to_raw_handle().map_err(|e| e.into_pyexception(vm))?;
            let mut li = winnt::LARGE_INTEGER::default();
            *li.QuadPart_mut() = position;
            let ret = fileapi::SetFilePointer(
                handle,
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
        dir_fd: DirFd<{ UTIME_DIR_FD as usize }>,
        #[pyarg(flatten)]
        follow_symlinks: FollowSymlinks,
    }

    #[pyfunction]
    fn utime(args: UtimeArgs, vm: &VirtualMachine) -> PyResult<()> {
        let parse_tup = |tup: &PyTuple| -> Option<(PyObjectRef, PyObjectRef)> {
            let tup = tup.as_slice();
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
                                    obj.class().name(),
                                    divmod.class().name()
                                ))
                            })?;
                    let secs = vm.to_index(&div)?.try_to_primitive(vm)?;
                    let ns = vm.to_index(&rem)?.try_to_primitive(vm)?;
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
        utime_impl(args.path, acc, modif, args.dir_fd, args.follow_symlinks, vm)
    }

    fn utime_impl(
        path: PyPathLike,
        acc: Duration,
        modif: Duration,
        dir_fd: DirFd<{ UTIME_DIR_FD as usize }>,
        _follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        #[cfg(any(target_os = "wasi", unix))]
        {
            #[cfg(not(target_os = "redox"))]
            {
                let path = path.into_cstring(vm)?;

                let ts = |d: Duration| libc::timespec {
                    tv_sec: d.as_secs() as _,
                    tv_nsec: d.subsec_nanos() as _,
                };
                let times = [ts(acc), ts(modif)];

                let ret = unsafe {
                    libc::utimensat(
                        dir_fd.fd().0,
                        path.as_ptr(),
                        times.as_ptr(),
                        if _follow_symlinks.0 {
                            0
                        } else {
                            libc::AT_SYMLINK_NOFOLLOW
                        },
                    )
                };
                if ret < 0 {
                    Err(errno_err(vm))
                } else {
                    Ok(())
                }
            }
            #[cfg(target_os = "redox")]
            {
                let [] = dir_fd.0;

                let tv = |d: Duration| libc::timeval {
                    tv_sec: d.as_secs() as _,
                    tv_usec: d.as_micros() as _,
                };
                nix::sys::stat::utimes(path.as_ref(), &tv(acc).into(), &tv(modif).into())
                    .map_err(|err| err.into_pyexception(vm))
            }
        }
        #[cfg(windows)]
        {
            use std::fs::OpenOptions;
            use std::os::windows::prelude::*;
            use winapi::shared::minwindef::{DWORD, FILETIME};
            use winapi::um::fileapi::SetFileTime;

            let [] = dir_fd.0;

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

    #[cfg(all(any(unix, windows), not(target_os = "redox")))]
    #[pyattr]
    #[pyclass(module = "os", name = "times_result")]
    #[derive(Debug, PyStructSequence)]
    struct TimesResult {
        pub user: f64,
        pub system: f64,
        pub children_user: f64,
        pub children_system: f64,
        pub elapsed: f64,
    }

    #[cfg(all(any(unix, windows), not(target_os = "redox")))]
    #[pyimpl(with(PyStructSequence))]
    impl TimesResult {}

    #[cfg(all(any(unix, windows), not(target_os = "redox")))]
    #[pyfunction]
    fn times(vm: &VirtualMachine) -> PyResult {
        #[cfg(windows)]
        {
            use winapi::shared::minwindef::FILETIME;
            use winapi::um::processthreadsapi::{GetCurrentProcess, GetProcessTimes};

            let mut _create = FILETIME::default();
            let mut _exit = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();

            unsafe {
                let h_proc = GetCurrentProcess();
                GetProcessTimes(h_proc, &mut _create, &mut _exit, &mut kernel, &mut user);
            }

            let times_result = TimesResult {
                user: user.dwHighDateTime as f64 * 429.4967296 + user.dwLowDateTime as f64 * 1e-7,
                system: kernel.dwHighDateTime as f64 * 429.4967296
                    + kernel.dwLowDateTime as f64 * 1e-7,
                children_user: 0.0,
                children_system: 0.0,
                elapsed: 0.0,
            };

            Ok(times_result.into_pyobject(vm))
        }
        #[cfg(unix)]
        {
            let mut t = libc::tms {
                tms_utime: 0,
                tms_stime: 0,
                tms_cutime: 0,
                tms_cstime: 0,
            };

            let tick_for_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
            let c = unsafe { libc::times(&mut t as *mut _) } as i64;

            if c == -1 {
                return Err(vm.new_os_error("Fail to get times".to_string()));
            }

            let times_result = TimesResult {
                user: t.tms_utime as f64 / tick_for_second,
                system: t.tms_stime as f64 / tick_for_second,
                children_user: t.tms_cutime as f64 / tick_for_second,
                children_system: t.tms_cstime as f64 / tick_for_second,
                elapsed: c as f64 / tick_for_second,
            };

            Ok(times_result.into_pyobject(vm))
        }
    }

    #[cfg(target_os = "linux")]
    #[derive(FromArgs)]
    struct CopyFileRangeArgs {
        #[pyarg(positional)]
        src: i32,
        #[pyarg(positional)]
        dst: i32,
        #[pyarg(positional)]
        count: i64,
        #[pyarg(any, default)]
        offset_src: Option<Offset>,
        #[pyarg(any, default)]
        offset_dst: Option<Offset>,
    }

    #[cfg(target_os = "linux")]
    #[pyfunction]
    fn copy_file_range(args: CopyFileRangeArgs, vm: &VirtualMachine) -> PyResult<usize> {
        use std::convert::{TryFrom, TryInto};
        let p_offset_src = args.offset_src.as_ref().map_or_else(std::ptr::null, |x| x);
        let p_offset_dst = args.offset_dst.as_ref().map_or_else(std::ptr::null, |x| x);
        let count: usize = args
            .count
            .try_into()
            .map_err(|_| vm.new_value_error("count should >= 0".to_string()))?;

        // The flags argument is provided to allow
        // for future extensions and currently must be to 0.
        let flags = 0u32;

        // Safety: p_offset_src and p_offset_dst is a unique pointer for offset_src and offset_dst respectively,
        // and will only be freed after this function ends.
        //
        // Why not use `libc::copy_file_range`: On `musl-libc`, `libc::copy_file_range` is not provided. Therefore
        // we use syscalls directly instead.
        let ret = unsafe {
            libc::syscall(
                libc::SYS_copy_file_range,
                args.src,
                p_offset_src as *mut i64,
                args.dst,
                p_offset_dst as *mut i64,
                count,
                flags,
            )
        };

        usize::try_from(ret).map_err(|_| errno_err(vm))
    }

    #[pyfunction]
    fn strerror(e: i32) -> String {
        unsafe { ffi::CStr::from_ptr(libc::strerror(e)) }
            .to_string_lossy()
            .into_owned()
    }

    #[pyfunction]
    pub fn ftruncate(fd: i32, length: Offset, vm: &VirtualMachine) -> PyResult<()> {
        Fd(fd).ftruncate(length).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn truncate(path: PyObjectRef, length: Offset, vm: &VirtualMachine) -> PyResult<()> {
        if let Ok(fd) = i32::try_from_borrowed_object(vm, &path) {
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

    #[cfg(all(unix, not(any(target_os = "redox", target_os = "android"))))]
    #[pyfunction]
    fn getloadavg(vm: &VirtualMachine) -> PyResult<(f64, f64, f64)> {
        let mut loadavg = [0f64; 3];

        // Safety: loadavg is on stack and only write by `getloadavg` and are freed
        // after this function ends.
        unsafe {
            if libc::getloadavg(&mut loadavg[0] as *mut f64, 3) != 3 {
                return Err(vm.new_os_error("Load averages are unobtainable".to_string()));
            }
        }

        Ok((loadavg[0], loadavg[1], loadavg[2]))
    }

    #[pyattr]
    #[pyclass(module = "os", name = "terminal_size")]
    #[derive(PyStructSequence)]
    #[allow(dead_code)]
    pub(crate) struct PyTerminalSize {
        pub columns: usize,
        pub lines: usize,
    }
    #[pyimpl(with(PyStructSequence))]
    impl PyTerminalSize {}

    #[pyattr]
    #[pyclass(module = "os", name = "uname_result")]
    #[derive(Debug, PyStructSequence)]
    pub(crate) struct UnameResult {
        pub sysname: String,
        pub nodename: String,
        pub release: String,
        pub version: String,
        pub machine: String,
    }

    #[pyimpl(with(PyStructSequence))]
    impl UnameResult {}

    pub(super) fn support_funcs() -> Vec<SupportFunc> {
        let mut supports = super::platform::module::support_funcs();
        supports.extend(vec![
            SupportFunc::new("open", Some(false), Some(OPEN_DIR_FD), Some(false)),
            SupportFunc::new("access", Some(false), Some(false), None),
            SupportFunc::new("chdir", None, Some(false), Some(false)),
            // chflags Some, None Some
            SupportFunc::new("listdir", Some(LISTDIR_FD), Some(false), Some(false)),
            SupportFunc::new("mkdir", Some(false), Some(MKDIR_DIR_FD), Some(false)),
            // mkfifo Some Some None
            // mknod Some Some None
            SupportFunc::new("readlink", Some(false), None, Some(false)),
            SupportFunc::new("remove", Some(false), None, Some(false)),
            SupportFunc::new("unlink", Some(false), None, Some(false)),
            SupportFunc::new("rename", Some(false), None, Some(false)),
            SupportFunc::new("replace", Some(false), None, Some(false)), // TODO: Fix replace
            SupportFunc::new("rmdir", Some(false), None, Some(false)),
            SupportFunc::new("scandir", None, Some(false), Some(false)),
            SupportFunc::new("stat", Some(true), Some(STAT_DIR_FD), Some(true)),
            SupportFunc::new("fstat", Some(true), Some(STAT_DIR_FD), Some(true)),
            SupportFunc::new("symlink", Some(false), Some(SYMLINK_DIR_FD), Some(false)),
            SupportFunc::new("truncate", Some(true), Some(false), Some(false)),
            SupportFunc::new(
                "utime",
                Some(false),
                Some(UTIME_DIR_FD),
                Some(cfg!(all(unix, not(target_os = "redox")))),
            ),
        ]);
        supports
    }
}
pub(crate) use _os::{ftruncate, isatty, lseek};

pub(crate) struct SupportFunc {
    name: &'static str,
    // realistically, each of these is just a bool of "is this function in the supports_* set".
    // However, None marks that the function maybe _should_ support fd/dir_fd/follow_symlinks, but
    // we haven't implemented it yet.
    fd: Option<bool>,
    dir_fd: Option<bool>,
    follow_symlinks: Option<bool>,
}

impl<'a> SupportFunc {
    pub(crate) fn new(
        name: &'static str,
        fd: Option<bool>,
        dir_fd: Option<bool>,
        follow_symlinks: Option<bool>,
    ) -> Self {
        Self {
            name,
            fd,
            dir_fd,
            follow_symlinks,
        }
    }
}

pub fn extend_module(vm: &VirtualMachine, module: &PyObjectRef) {
    _os::extend_module(vm, module);

    let support_funcs = _os::support_funcs();
    let supports_fd = PySet::default().into_ref(vm);
    let supports_dir_fd = PySet::default().into_ref(vm);
    let supports_follow_symlinks = PySet::default().into_ref(vm);
    for support in support_funcs {
        let func_obj = vm.get_attribute(module.clone(), support.name).unwrap();
        if support.fd.unwrap_or(false) {
            supports_fd.clone().add(func_obj.clone(), vm).unwrap();
        }
        if support.dir_fd.unwrap_or(false) {
            supports_dir_fd.clone().add(func_obj.clone(), vm).unwrap();
        }
        if support.follow_symlinks.unwrap_or(false) {
            supports_follow_symlinks.clone().add(func_obj, vm).unwrap();
        }
    }

    extend_module!(vm, module, {
        "supports_fd" => supports_fd.into_object(),
        "supports_dir_fd" => supports_dir_fd.into_object(),
        "supports_follow_symlinks" => supports_follow_symlinks.into_object(),
        "error" => vm.ctx.exceptions.os_error.clone(),
    });
}
pub(crate) use _os::os_open as open;

#[cfg(not(windows))]
use super::posix as platform;

#[cfg(windows)]
use super::nt as platform;

pub(crate) use platform::module::MODULE_NAME;

#[cfg(not(all(windows, target_env = "msvc")))]
#[macro_export]
macro_rules! suppress_iph {
    ($e:expr) => {
        $e
    };
}
