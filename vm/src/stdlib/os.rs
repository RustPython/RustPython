use crate::{
    builtins::{PyBaseExceptionRef, PyBytes, PyBytesRef, PyInt, PySet, PyStr, PyStrRef},
    common::crt_fd::Fd,
    convert::{IntoPyException, ToPyObject},
    function::{ArgumentError, FromArgs, FuncArgs},
    identifier,
    protocol::PyBuffer,
    AsObject, PyObject, PyObjectRef, PyPayload, PyResult, TryFromBorrowedObject, TryFromObject,
    VirtualMachine,
};
use std::{
    ffi, fs, io,
    path::{Path, PathBuf},
};

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
                OutputMode::String => path_as_string(path).map(|s| vm.ctx.new_str(s).into()),
                OutputMode::Bytes => {
                    #[cfg(any(unix, target_os = "wasi"))]
                    {
                        use ffi_ext::OsStringExt;
                        Ok(vm.ctx.new_bytes(path.into_os_string().into_vec()).into())
                    }
                    #[cfg(windows)]
                    {
                        path_as_string(path).map(|s| vm.ctx.new_bytes(s.into_bytes()).into())
                    }
                }
            }
        }
        inner(self, path.into(), vm)
    }
}

#[derive(Clone)]
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

    #[cfg(windows)]
    pub fn into_bytes(self) -> Vec<u8> {
        self.path.to_string_lossy().to_string().into_bytes()
    }

    // #[cfg(any(unix, target_os = "wasi"))]
    pub fn into_cstring(self, vm: &VirtualMachine) -> PyResult<ffi::CString> {
        ffi::CString::new(self.into_bytes()).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(windows)]
    pub fn to_widecstring(&self, vm: &VirtualMachine) -> PyResult<widestring::WideCString> {
        widestring::WideCString::from_os_str(&self.path).map_err(|err| err.into_pyexception(vm))
    }

    pub fn filename(&self, vm: &VirtualMachine) -> PyResult {
        self.mode.process_path(self.path.clone(), vm)
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
    pub fn try_from(obj: PyObjectRef, check_for_nul: bool, vm: &VirtualMachine) -> PyResult<Self> {
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
        let method =
            vm.get_method_or_type_error(obj.clone(), identifier!(vm, __fspath__), || {
                format!(
                    "should be string, bytes, os.PathLike or integer, not {}",
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

    pub fn as_bytes(&self) -> &[u8] {
        // TODO: FS encodings
        match self {
            FsPath::Str(s) => s.as_str().as_bytes(),
            FsPath::Bytes(b) => b.as_bytes(),
        }
    }
}

impl ToPyObject for FsPath {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::Str(s) => s.into(),
            Self::Bytes(b) => b.into(),
        }
    }
}

impl TryFromObject for PyPathLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        // path_converter in CPython
        let obj = match PyBuffer::try_from_borrowed_object(vm, &obj) {
            Ok(buffer) => {
                let mut bytes = vec![];
                buffer.append_to(&mut bytes);
                PyBytes::from(bytes).to_pyobject(vm)
            }
            Err(_) => obj,
        };
        let fs_path = FsPath::try_from(obj, true, vm)?;
        fs_path.to_pathlike(vm)
    }
}

#[derive(Clone)]
pub(crate) enum PathOrFd {
    Path(PyPathLike),
    Fd(i32),
}

impl TryFromObject for PathOrFd {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let r = match obj.downcast::<PyInt>() {
            Ok(int) => Self::Fd(int.try_to_primitive(vm)?),
            Err(obj) => Self::Path(obj.try_into_value(vm)?),
        };
        Ok(r)
    }
}

impl From<PyPathLike> for PathOrFd {
    fn from(path: PyPathLike) -> Self {
        Self::Path(path)
    }
}

impl PathOrFd {
    pub fn filename(&self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            PathOrFd::Path(path) => path.filename(vm).unwrap_or_else(|_| vm.ctx.none()),
            PathOrFd::Fd(fd) => vm.ctx.new_int(*fd).into(),
        }
    }
}

#[cfg(unix)]
impl IntoPyException for nix::Error {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        io::Error::from(self).into_pyexception(vm)
    }
}

// TODO: preserve the input `PyObjectRef` of filename and filename2 (Failing check `self.assertIs(err.filename, name, str(func)`)
pub struct IOErrorBuilder {
    error: io::Error,
    filename: Option<PathOrFd>,
    filename2: Option<PathOrFd>,
}

impl IOErrorBuilder {
    pub fn new(error: io::Error) -> Self {
        Self {
            error,
            filename: None,
            filename2: None,
        }
    }
    pub(crate) fn filename(mut self, filename: impl Into<PathOrFd>) -> Self {
        self.filename.replace(filename.into());
        self
    }
    pub(crate) fn filename2(mut self, filename: impl Into<PathOrFd>) -> Self {
        self.filename2.replace(filename.into());
        self
    }
}

impl IntoPyException for IOErrorBuilder {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let excp = self.error.into_pyexception(vm);

        if let Some(filename) = self.filename {
            excp.as_object()
                .set_attr("filename", filename.filename(vm), vm)
                .unwrap();
        }
        if let Some(filename2) = self.filename2 {
            excp.as_object()
                .set_attr("filename2", filename2.filename(vm), vm)
                .unwrap();
        }
        excp
    }
}

/// Convert the error stored in the `errno` variable into an Exception
#[inline]
pub fn errno_err(vm: &VirtualMachine) -> PyBaseExceptionRef {
    crate::common::os::errno().into_pyexception(vm)
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
                let fd = o.try_index_opt(vm).unwrap_or_else(|| {
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
        errno_err, DirFd, FollowSymlinks, FsPath, IOErrorBuilder, OutputMode, PathOrFd, PyPathLike,
        SupportFunc,
    };
    use crate::common::lock::{OnceCell, PyRwLock};
    use crate::{
        builtins::{
            PyBytesRef, PyGenericAlias, PyIntRef, PyStrRef, PyTuple, PyTupleRef, PyTypeRef,
        },
        common::crt_fd::{Fd, Offset},
        common::suppress_iph,
        convert::{IntoPyException, ToPyObject},
        function::Either,
        function::{ArgBytesLike, FuncArgs, OptionalArg},
        protocol::PyIterReturn,
        recursion::ReprGuard,
        types::{IterNext, IterNextIterable, PyStructSequence},
        vm::VirtualMachine,
        AsObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use itertools::Itertools;
    use std::{
        env, ffi, fs,
        fs::OpenOptions,
        io::{self, Read, Write},
        path::PathBuf,
        time::{Duration, SystemTime},
    };

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
        path: PyPathLike,
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
            let name = name.clone().into_cstring(vm)?;
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
        fd.map(|fd| fd.0)
            .map_err(|e| IOErrorBuilder::new(e).filename(name).into_pyexception(vm))
    }

    #[pyfunction]
    fn fsync(fd: i32, vm: &VirtualMachine) -> PyResult<()> {
        Fd(fd).fsync().map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn read(fd: i32, n: usize, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
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

        Ok(vm.ctx.new_int(written).into())
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
        res.map_err(|e| IOErrorBuilder::new(e).filename(path).into_pyexception(vm))
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
        fs::remove_dir(&path)
            .map_err(|e| IOErrorBuilder::new(e).filename(path).into_pyexception(vm))
    }

    const LISTDIR_FD: bool = cfg!(all(unix, not(target_os = "redox")));

    #[pyfunction]
    fn listdir(path: OptionalArg<PathOrFd>, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let path = path.unwrap_or_else(|| PathOrFd::Path(PyPathLike::new_str(".")));
        let list = match path {
            PathOrFd::Path(path) => {
                let dir_iter = fs::read_dir(&path).map_err(|err| err.into_pyexception(vm))?;
                dir_iter
                    .map(|entry| match entry {
                        Ok(entry_path) => path.mode.process_path(entry_path.file_name(), vm),
                        Err(e) => Err(IOErrorBuilder::new(e)
                            .filename(path.clone())
                            .into_pyexception(vm)),
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
        Ok(list)
    }

    fn pyref_as_str<'a>(
        obj: &'a Either<PyStrRef, PyBytesRef>,
        vm: &VirtualMachine,
    ) -> PyResult<&'a str> {
        Ok(match obj {
            Either::A(ref s) => s.as_str(),
            Either::B(ref b) => super::bytes_as_osstr(b.as_bytes(), vm)?
                .to_str()
                .ok_or_else(|| {
                    vm.new_unicode_decode_error("can't decode bytes for utf-8".to_owned())
                })?,
        })
    }

    #[pyfunction]
    fn putenv(
        key: Either<PyStrRef, PyBytesRef>,
        value: Either<PyStrRef, PyBytesRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let key = pyref_as_str(&key, vm)?;
        let value = pyref_as_str(&value, vm)?;
        if key.contains('\0') || value.contains('\0') {
            return Err(vm.new_value_error("embedded null byte".to_string()));
        }
        if key.is_empty() || key.contains('=') {
            return Err(vm.new_value_error("illegal environment variable name".to_string()));
        }
        env::set_var(key, value);
        Ok(())
    }

    #[pyfunction]
    fn unsetenv(key: Either<PyStrRef, PyBytesRef>, vm: &VirtualMachine) -> PyResult<()> {
        let key = pyref_as_str(&key, vm)?;
        if key.contains('\0') {
            return Err(vm.new_value_error("embedded null byte".to_string()));
        }
        if key.is_empty() || key.contains('=') {
            return Err(vm.new_value_error("illegal environment variable name".to_string()));
        }
        env::remove_var(key);
        Ok(())
    }

    #[pyfunction]
    fn readlink(path: PyPathLike, dir_fd: DirFd<0>, vm: &VirtualMachine) -> PyResult {
        let mode = path.mode;
        let [] = dir_fd.0;
        let path = fs::read_link(&path)
            .map_err(|err| IOErrorBuilder::new(err).filename(path).into_pyexception(vm))?;
        mode.process_path(path, vm)
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct DirEntry {
        file_name: std::ffi::OsString,
        pathval: PathBuf,
        file_type: io::Result<fs::FileType>,
        mode: OutputMode,
        stat: OnceCell<PyObjectRef>,
        lstat: OnceCell<PyObjectRef>,
        #[cfg(unix)]
        ino: AtomicCell<u64>,
        #[cfg(not(unix))]
        ino: AtomicCell<Option<u64>>,
    }

    #[pyclass]
    impl DirEntry {
        #[pygetset]
        fn name(&self, vm: &VirtualMachine) -> PyResult {
            self.mode.process_path(&self.file_name, vm)
        }

        #[pygetset]
        fn path(&self, vm: &VirtualMachine) -> PyResult {
            self.mode.process_path(&self.pathval, vm)
        }

        fn perform_on_metadata(
            &self,
            follow_symlinks: FollowSymlinks,
            action: fn(fs::Metadata) -> bool,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            match super::fs_metadata(&self.pathval, follow_symlinks.0) {
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
                .file_type
                .as_ref()
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
                        path: self.pathval.clone(),
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
                            path: self.pathval.clone(),
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
            Ok(self.ino.load())
        }

        #[pymethod(magic)]
        fn fspath(&self, vm: &VirtualMachine) -> PyResult {
            self.path(vm)
        }

        #[pymethod(magic)]
        fn repr(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            let name = match zelf.get_attr("name", vm) {
                Ok(name) => Some(name),
                Err(e)
                    if e.fast_isinstance(vm.ctx.exceptions.attribute_error)
                        || e.fast_isinstance(vm.ctx.exceptions.value_error) =>
                {
                    None
                }
                Err(e) => return Err(e),
            };
            if let Some(name) = name {
                if let Some(_guard) = ReprGuard::enter(vm, &zelf) {
                    let repr = name.repr(vm)?;
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

        #[pyclassmethod(magic)]
        fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
            PyGenericAlias::new(cls, args, vm)
        }
    }

    #[pyattr]
    #[pyclass(name = "ScandirIter")]
    #[derive(Debug, PyPayload)]
    struct ScandirIterator {
        entries: PyRwLock<Option<fs::ReadDir>>,
        mode: OutputMode,
    }

    #[pyclass(with(IterNext))]
    impl ScandirIterator {
        #[pymethod]
        fn close(&self) {
            let entryref: &mut Option<fs::ReadDir> = &mut self.entries.write();
            let _dropped = entryref.take();
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
    impl IterNextIterable for ScandirIterator {}
    impl IterNext for ScandirIterator {
        fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let entryref: &mut Option<fs::ReadDir> = &mut zelf.entries.write();

            match entryref {
                None => Ok(PyIterReturn::StopIteration(None)),
                Some(inner) => match inner.next() {
                    Some(entry) => match entry {
                        Ok(entry) => {
                            #[cfg(unix)]
                            let ino = {
                                use std::os::unix::fs::DirEntryExt;
                                entry.ino()
                            };
                            #[cfg(not(unix))]
                            let ino = None;

                            Ok(PyIterReturn::Return(
                                DirEntry {
                                    file_name: entry.file_name(),
                                    pathval: entry.path(),
                                    file_type: entry.file_type(),
                                    mode: zelf.mode,
                                    lstat: OnceCell::new(),
                                    stat: OnceCell::new(),
                                    ino: AtomicCell::new(ino),
                                }
                                .into_ref(vm)
                                .into(),
                            ))
                        }
                        Err(err) => Err(err.into_pyexception(vm)),
                    },
                    None => {
                        let _dropped = entryref.take();
                        Ok(PyIterReturn::StopIteration(None))
                    }
                },
            }
        }
    }

    #[pyfunction]
    fn scandir(path: OptionalArg<PyPathLike>, vm: &VirtualMachine) -> PyResult {
        let path = path.unwrap_or_else(|| PyPathLike::new_str("."));
        let entries = fs::read_dir(path.path).map_err(|err| err.into_pyexception(vm))?;
        Ok(ScandirIterator {
            entries: PyRwLock::new(Some(entries)),
            mode: path.mode,
        }
        .into_ref(vm)
        .into())
    }

    #[pyattr]
    #[pyclass(module = "os", name = "stat_result")]
    #[derive(Debug, PyStructSequence, FromArgs)]
    struct StatResult {
        pub st_mode: PyIntRef,
        pub st_ino: PyIntRef,
        pub st_dev: PyIntRef,
        pub st_nlink: PyIntRef,
        pub st_uid: PyIntRef,
        pub st_gid: PyIntRef,
        pub st_size: PyIntRef,
        // TODO: unnamed structsequence fields
        #[pyarg(positional, default)]
        pub __st_atime_int: libc::time_t,
        #[pyarg(positional, default)]
        pub __st_mtime_int: libc::time_t,
        #[pyarg(positional, default)]
        pub __st_ctime_int: libc::time_t,
        #[pyarg(any, default)]
        pub st_atime: f64,
        #[pyarg(any, default)]
        pub st_mtime: f64,
        #[pyarg(any, default)]
        pub st_ctime: f64,
        #[pyarg(any, default)]
        pub st_atime_ns: i128,
        #[pyarg(any, default)]
        pub st_mtime_ns: i128,
        #[pyarg(any, default)]
        pub st_ctime_ns: i128,
    }

    #[pyclass(with(PyStructSequence))]
    impl StatResult {
        fn from_stat(stat: &StatStruct, vm: &VirtualMachine) -> Self {
            let (atime, mtime, ctime);
            #[cfg(any(unix, windows))]
            #[cfg(not(target_os = "netbsd"))]
            {
                atime = (stat.st_atime, stat.st_atime_nsec);
                mtime = (stat.st_mtime, stat.st_mtime_nsec);
                ctime = (stat.st_ctime, stat.st_ctime_nsec);
            }
            #[cfg(target_os = "netbsd")]
            {
                atime = (stat.st_atime, stat.st_atimensec);
                mtime = (stat.st_mtime, stat.st_mtimensec);
                ctime = (stat.st_ctime, stat.st_ctimensec);
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
                st_mode: vm.new_pyref(stat.st_mode),
                st_ino: vm.new_pyref(stat.st_ino),
                st_dev: vm.new_pyref(stat.st_dev),
                st_nlink: vm.new_pyref(stat.st_nlink),
                st_uid: vm.new_pyref(stat.st_uid),
                st_gid: vm.new_pyref(stat.st_gid),
                st_size: vm.new_pyref(stat.st_size),
                __st_atime_int: atime.0,
                __st_mtime_int: mtime.0,
                __st_ctime_int: ctime.0,
                st_atime: to_f64(atime),
                st_mtime: to_f64(mtime),
                st_ctime: to_f64(ctime),
                st_atime_ns: to_ns(atime),
                st_mtime_ns: to_ns(mtime),
                st_ctime_ns: to_ns(ctime),
            }
        }

        #[pyslot]
        fn slot_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
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

            let args: FuncArgs = flatten_args(&args.args).into();

            let stat: StatResult = args.bind(vm)?;
            Ok(stat.to_pyobject(vm))
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
            PathOrFd::Path(path) => super::fs_metadata(path, follow_symlinks.0)?,
            PathOrFd::Fd(fno) => {
                use std::os::windows::io::FromRawHandle;
                let handle = Fd(fno).to_raw_handle()?;
                let file =
                    std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(handle) });
                file.metadata()?
            }
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
        let stat = stat_inner(file.clone(), dir_fd, follow_symlinks)
            .map_err(|e| IOErrorBuilder::new(e).filename(file).into_pyexception(vm))?
            .ok_or_else(|| crate::exceptions::cstring_error(vm))?;
        Ok(StatResult::from_stat(&stat, vm).to_pyobject(vm))
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
        env::current_dir().map_err(|err| err.into_pyexception(vm))
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
        env::set_current_dir(&path.path)
            .map_err(|err| IOErrorBuilder::new(err).filename(path).into_pyexception(vm))
    }

    #[pyfunction]
    fn fspath(path: PyObjectRef, vm: &VirtualMachine) -> PyResult<FsPath> {
        super::FsPath::try_from(path, false, vm)
    }

    #[pyfunction]
    #[pyfunction(name = "replace")]
    fn rename(src: PyPathLike, dst: PyPathLike, vm: &VirtualMachine) -> PyResult<()> {
        fs::rename(&src.path, &dst.path).map_err(|err| {
            IOErrorBuilder::new(err)
                .filename(src)
                .filename2(dst)
                .into_pyexception(vm)
        })
    }

    #[pyfunction]
    fn getpid(vm: &VirtualMachine) -> PyObjectRef {
        let pid = std::process::id();
        vm.ctx.new_int(pid).into()
    }

    #[pyfunction]
    fn cpu_count(vm: &VirtualMachine) -> PyObjectRef {
        let cpu_count = num_cpus::get();
        vm.ctx.new_int(cpu_count).into()
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
    fn urandom(size: isize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        if size < 0 {
            return Err(vm.new_value_error("negative argument not allowed".to_owned()));
        }
        let mut buf = vec![0u8; size as usize];
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
        fs::hard_link(&src.path, &dst.path).map_err(|err| {
            IOErrorBuilder::new(err)
                .filename(src)
                .filename2(dst)
                .into_pyexception(vm)
        })
    }

    #[derive(FromArgs)]
    struct UtimeArgs {
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
                (a.try_into_value(vm)?, m.try_into_value(vm)?)
            }
            (None, Some(ns)) => {
                let (a, m) = parse_tup(&ns).ok_or_else(|| {
                    vm.new_type_error("utime: 'ns' must be a tuple of two ints".to_owned())
                })?;
                let ns_in_sec: PyObjectRef = vm.ctx.new_int(1_000_000_000).into();
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
                    let secs = div.try_index(vm)?.try_to_primitive(vm)?;
                    let ns = rem.try_index(vm)?.try_to_primitive(vm)?;
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
            use std::{fs::OpenOptions, os::windows::prelude::*};
            use winapi::{
                shared::minwindef::{DWORD, FILETIME},
                um::fileapi::SetFileTime,
            };

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
    #[pyclass(with(PyStructSequence))]
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

            Ok(times_result.to_pyobject(vm))
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
            let c = unsafe { libc::times(&mut t as *mut _) };

            // XXX: The signedness of `clock_t` varies from platform to platform.
            if c == (-1i8) as libc::clock_t {
                return Err(vm.new_os_error("Fail to get times".to_string()));
            }

            let times_result = TimesResult {
                user: t.tms_utime as f64 / tick_for_second,
                system: t.tms_stime as f64 / tick_for_second,
                children_user: t.tms_cutime as f64 / tick_for_second,
                children_system: t.tms_cstime as f64 / tick_for_second,
                elapsed: c as f64 / tick_for_second,
            };

            Ok(times_result.to_pyobject(vm))
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
        if let Ok(fd) = path.try_to_value(vm) {
            return ftruncate(fd, length, vm);
        }
        let path = PyPathLike::try_from_object(vm, path)?;
        // TODO: just call libc::truncate() on POSIX
        let f = OpenOptions::new()
            .write(true)
            .open(path)
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

    #[cfg(any(unix, windows))]
    #[pyfunction]
    fn waitstatus_to_exitcode(status: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let status = u32::try_from(status)
            .map_err(|_| vm.new_value_error(format!("invalid WEXITSTATUS: {status}")))?;

        cfg_if::cfg_if! {
            if #[cfg(not(windows))] {
                let status = status as libc::c_int;
                if libc::WIFEXITED(status) {
                    return Ok(libc::WEXITSTATUS(status));
                }

                if libc::WIFSIGNALED(status) {
                    return Ok(-libc::WTERMSIG(status));
                }

                Err(vm.new_value_error(format!("Invalid wait status: {status}")))
            } else {
                i32::try_from(status.rotate_right(8))
                    .map_err(|_| vm.new_value_error(format!("invalid wait status: {}", status)))
            }
        }
    }

    #[pyfunction]
    fn device_encoding(fd: i32, _vm: &VirtualMachine) -> PyResult<Option<String>> {
        if !isatty(fd) {
            return Ok(None);
        }

        cfg_if::cfg_if! {
            if #[cfg(target_os = "android")] {
                Ok(Some("UTF-8".to_owned()))
            } else if #[cfg(windows)] {
                let cp = match fd {
                    0 => unsafe { winapi::um::consoleapi::GetConsoleCP() },
                    1 | 2 => unsafe { winapi::um::consoleapi::GetConsoleOutputCP() },
                    _ => 0,
                };

                Ok(Some(format!("cp{}", cp)))
            } else {
                let encoding = unsafe {
                    let encoding = libc::nl_langinfo(libc::CODESET);
                    if encoding.is_null() || encoding.read() == '\0' as libc::c_char {
                        "UTF-8".to_owned()
                    } else {
                        ffi::CStr::from_ptr(encoding).to_string_lossy().into_owned()
                    }
                };

                Ok(Some(encoding))
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "os", name = "terminal_size")]
    #[derive(PyStructSequence)]
    #[allow(dead_code)]
    pub(crate) struct PyTerminalSize {
        pub columns: usize,
        pub lines: usize,
    }
    #[pyclass(with(PyStructSequence))]
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

    #[pyclass(with(PyStructSequence))]
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

impl SupportFunc {
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

pub fn extend_module(vm: &VirtualMachine, module: &PyObject) {
    _os::extend_module(vm, module);

    let support_funcs = _os::support_funcs();
    let supports_fd = PySet::default().into_ref(vm);
    let supports_dir_fd = PySet::default().into_ref(vm);
    let supports_follow_symlinks = PySet::default().into_ref(vm);
    for support in support_funcs {
        let func_obj = module.to_owned().get_attr(support.name, vm).unwrap();
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
        "supports_fd" => supports_fd,
        "supports_dir_fd" => supports_dir_fd,
        "supports_follow_symlinks" => supports_follow_symlinks,
        "error" => vm.ctx.exceptions.os_error.to_owned(),
    });
}
pub(crate) use _os::os_open as open;

#[cfg(not(windows))]
use super::posix as platform;

#[cfg(windows)]
use super::nt as platform;

pub(crate) use platform::module::MODULE_NAME;
