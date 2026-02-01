// spell-checker:disable

use crate::{
    AsObject, Py, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyModule, PySet},
    common::crt_fd,
    convert::{IntoPyException, ToPyException, ToPyObject},
    function::{ArgumentError, FromArgs, FuncArgs},
};
use std::{fs, io, path::Path};

pub(crate) fn fs_metadata<P: AsRef<Path>>(
    path: P,
    follow_symlink: bool,
) -> io::Result<fs::Metadata> {
    if follow_symlink {
        fs::metadata(path.as_ref())
    } else {
        fs::symlink_metadata(path.as_ref())
    }
}

#[cfg(unix)]
impl crate::convert::IntoPyException for nix::Error {
    fn into_pyexception(self, vm: &VirtualMachine) -> crate::builtins::PyBaseExceptionRef {
        io::Error::from(self).into_pyexception(vm)
    }
}

#[cfg(unix)]
impl crate::convert::IntoPyException for rustix::io::Errno {
    fn into_pyexception(self, vm: &VirtualMachine) -> crate::builtins::PyBaseExceptionRef {
        io::Error::from(self).into_pyexception(vm)
    }
}

#[allow(dead_code)]
#[derive(FromArgs, Default)]
pub struct TargetIsDirectory {
    #[pyarg(any, default = false)]
    pub(crate) target_is_directory: bool,
}

cfg_if::cfg_if! {
    if #[cfg(all(any(unix, target_os = "wasi"), not(target_os = "redox")))] {
        use libc::AT_FDCWD;
    } else {
        const AT_FDCWD: i32 = -100;
    }
}
const DEFAULT_DIR_FD: crt_fd::Borrowed<'static> = unsafe { crt_fd::Borrowed::borrow_raw(AT_FDCWD) };

// XXX: AVAILABLE should be a bool, but we can't yet have it as a bool and just cast it to usize
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct DirFd<'fd, const AVAILABLE: usize>(pub(crate) [crt_fd::Borrowed<'fd>; AVAILABLE]);

impl<const AVAILABLE: usize> Default for DirFd<'_, AVAILABLE> {
    fn default() -> Self {
        Self([DEFAULT_DIR_FD; AVAILABLE])
    }
}

// not used on all platforms
#[allow(unused)]
impl<'fd> DirFd<'fd, 1> {
    #[inline(always)]
    pub(crate) fn get_opt(self) -> Option<crt_fd::Borrowed<'fd>> {
        let [fd] = self.0;
        (fd != DEFAULT_DIR_FD).then_some(fd)
    }

    #[inline]
    pub(crate) fn raw_opt(self) -> Option<i32> {
        self.get_opt().map(|fd| fd.as_raw())
    }

    #[inline(always)]
    pub(crate) const fn get(self) -> crt_fd::Borrowed<'fd> {
        let [fd] = self.0;
        fd
    }
}

impl<const AVAILABLE: usize> FromArgs for DirFd<'_, AVAILABLE> {
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let fd = match args.take_keyword("dir_fd") {
            Some(o) if vm.is_none(&o) => Ok(DEFAULT_DIR_FD),
            None => Ok(DEFAULT_DIR_FD),
            Some(o) => {
                let fd = o.try_index_opt(vm).unwrap_or_else(|| {
                    Err(vm.new_type_error(format!(
                        "argument should be integer or None, not {}",
                        o.class().name()
                    )))
                })?;
                let fd = fd.try_to_primitive(vm)?;
                unsafe { crt_fd::Borrowed::try_borrow_raw(fd) }
            }
        };
        if AVAILABLE == 0 && fd.as_ref().is_ok_and(|&fd| fd != DEFAULT_DIR_FD) {
            return Err(vm
                .new_not_implemented_error("dir_fd unavailable on this platform")
                .into());
        }
        let fd = fd.map_err(|e| e.to_pyexception(vm))?;
        Ok(Self([fd; AVAILABLE]))
    }
}

#[derive(FromArgs)]
pub(super) struct FollowSymlinks(
    #[pyarg(named, name = "follow_symlinks", default = true)] pub bool,
);

#[cfg(not(windows))]
fn bytes_as_os_str<'a>(b: &'a [u8], vm: &VirtualMachine) -> PyResult<&'a std::ffi::OsStr> {
    rustpython_common::os::bytes_as_os_str(b)
        .map_err(|_| vm.new_unicode_decode_error("can't decode path for utf-8"))
}

impl TryFromObject for crt_fd::Owned {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let fd = crt_fd::Raw::try_from_object(vm, obj)?;
        unsafe { crt_fd::Owned::try_from_raw(fd) }.map_err(|e| e.into_pyexception(vm))
    }
}

impl TryFromObject for crt_fd::Borrowed<'_> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let fd = crt_fd::Raw::try_from_object(vm, obj)?;
        unsafe { crt_fd::Borrowed::try_borrow_raw(fd) }.map_err(|e| e.into_pyexception(vm))
    }
}

impl ToPyObject for crt_fd::Owned {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        self.into_raw().to_pyobject(vm)
    }
}

impl ToPyObject for crt_fd::Borrowed<'_> {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        self.as_raw().to_pyobject(vm)
    }
}

#[pymodule(sub)]
pub(super) mod _os {
    use super::{DirFd, FollowSymlinks, SupportFunc};
    #[cfg(windows)]
    use crate::common::windows::ToWideString;
    #[cfg(any(unix, windows))]
    use crate::utils::ToCString;
    use crate::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
        builtins::{
            PyBytesRef, PyGenericAlias, PyIntRef, PyStrRef, PyTuple, PyTupleRef, PyTypeRef,
            ToOSErrorBuilder,
        },
        common::{
            crt_fd,
            fileutils::StatStruct,
            lock::{OnceCell, PyRwLock},
            suppress_iph,
        },
        convert::{IntoPyException, ToPyObject},
        exceptions::OSErrorBuilder,
        function::{ArgBytesLike, FsPath, FuncArgs, OptionalArg},
        ospath::{OsPath, OsPathOrFd, OutputMode, PathConverter},
        protocol::PyIterReturn,
        recursion::ReprGuard,
        types::{IterNext, Iterable, PyStructSequence, Representable, SelfIter},
        vm::VirtualMachine,
    };
    use core::time::Duration;
    use crossbeam_utils::atomic::AtomicCell;
    use itertools::Itertools;
    use std::{env, fs, fs::OpenOptions, io, path::PathBuf, time::SystemTime};

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

    // ST_RDONLY and ST_NOSUID flags for statvfs
    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyattr]
    const ST_RDONLY: libc::c_ulong = libc::ST_RDONLY;

    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyattr]
    const ST_NOSUID: libc::c_ulong = libc::ST_NOSUID;

    #[pyfunction]
    fn close(fileno: crt_fd::Owned) -> io::Result<()> {
        crt_fd::close(fileno)
    }

    #[pyfunction]
    fn closerange(fd_low: i32, fd_high: i32) {
        for fileno in fd_low..fd_high {
            if let Ok(fd) = unsafe { crt_fd::Owned::try_from_raw(fileno) } {
                drop(fd);
            }
        }
    }

    #[cfg(any(unix, windows, target_os = "wasi"))]
    #[derive(FromArgs)]
    struct OpenArgs<'fd> {
        path: OsPath,
        flags: i32,
        #[pyarg(any, default)]
        mode: Option<i32>,
        #[pyarg(flatten)]
        dir_fd: DirFd<'fd, { OPEN_DIR_FD as usize }>,
    }

    #[pyfunction]
    fn open(args: OpenArgs<'_>, vm: &VirtualMachine) -> PyResult<crt_fd::Owned> {
        os_open(args.path, args.flags, args.mode, args.dir_fd, vm)
    }

    #[cfg(any(unix, windows, target_os = "wasi"))]
    pub(crate) fn os_open(
        name: OsPath,
        flags: i32,
        mode: Option<i32>,
        dir_fd: DirFd<'_, { OPEN_DIR_FD as usize }>,
        vm: &VirtualMachine,
    ) -> PyResult<crt_fd::Owned> {
        let mode = mode.unwrap_or(0o777);
        #[cfg(windows)]
        let fd = {
            let [] = dir_fd.0;
            let name = name.to_wide_cstring(vm)?;
            let flags = flags | libc::O_NOINHERIT;
            crt_fd::wopen(&name, flags, mode)
        };
        #[cfg(not(windows))]
        let fd = {
            let name = name.clone().into_cstring(vm)?;
            #[cfg(not(target_os = "wasi"))]
            let flags = flags | libc::O_CLOEXEC;
            #[cfg(not(target_os = "redox"))]
            if let Some(dir_fd) = dir_fd.get_opt() {
                crt_fd::openat(dir_fd, &name, flags, mode)
            } else {
                crt_fd::open(&name, flags, mode)
            }
            #[cfg(target_os = "redox")]
            {
                let [] = dir_fd.0;
                crt_fd::open(&name, flags, mode)
            }
        };
        fd.map_err(|err| OSErrorBuilder::with_filename(&err, name, vm))
    }

    #[pyfunction]
    fn fsync(fd: crt_fd::Borrowed<'_>) -> io::Result<()> {
        crt_fd::fsync(fd)
    }

    #[pyfunction]
    fn read(fd: crt_fd::Borrowed<'_>, n: usize, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let mut buffer = vec![0u8; n];
        loop {
            match crt_fd::read(fd, &mut buffer) {
                Ok(n) => {
                    buffer.truncate(n);
                    return Ok(vm.ctx.new_bytes(buffer));
                }
                Err(e) if e.raw_os_error() == Some(libc::EINTR) => {
                    vm.check_signals()?;
                    continue;
                }
                Err(e) => return Err(e.into_pyexception(vm)),
            }
        }
    }

    #[pyfunction]
    fn write(fd: crt_fd::Borrowed<'_>, data: ArgBytesLike) -> io::Result<usize> {
        data.with_ref(|b| crt_fd::write(fd, b))
    }

    #[cfg(not(windows))]
    #[pyfunction]
    fn mkdir(
        path: OsPath,
        mode: OptionalArg<i32>,
        dir_fd: DirFd<'_, { MKDIR_DIR_FD as usize }>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mode = mode.unwrap_or(0o777);
        let c_path = path.clone().into_cstring(vm)?;
        #[cfg(not(target_os = "redox"))]
        if let Some(fd) = dir_fd.raw_opt() {
            let res = unsafe { libc::mkdirat(fd, c_path.as_ptr(), mode as _) };
            return if res < 0 {
                let err = crate::common::os::errno_io_error();
                Err(OSErrorBuilder::with_filename(&err, path, vm))
            } else {
                Ok(())
            };
        }
        #[cfg(target_os = "redox")]
        let [] = dir_fd.0;
        let res = unsafe { libc::mkdir(c_path.as_ptr(), mode as _) };
        if res < 0 {
            let err = crate::common::os::errno_io_error();
            return Err(OSErrorBuilder::with_filename(&err, path, vm));
        }
        Ok(())
    }

    #[pyfunction]
    fn mkdirs(path: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        fs::create_dir_all(path.as_str()).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn rmdir(path: OsPath, dir_fd: DirFd<'_, 0>, vm: &VirtualMachine) -> PyResult<()> {
        let [] = dir_fd.0;
        fs::remove_dir(&path).map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))
    }

    const LISTDIR_FD: bool = cfg!(all(unix, not(target_os = "redox")));

    #[pyfunction]
    fn listdir(
        path: OptionalArg<Option<OsPathOrFd<'_>>>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        let path = path
            .flatten()
            .unwrap_or_else(|| OsPathOrFd::Path(OsPath::new_str(".")));
        let list = match path {
            OsPathOrFd::Path(path) => {
                let dir_iter = match fs::read_dir(&path) {
                    Ok(iter) => iter,
                    Err(err) => {
                        return Err(OSErrorBuilder::with_filename(&err, path, vm));
                    }
                };
                let mode = path.mode();
                dir_iter
                    .map(|entry| match entry {
                        Ok(entry_path) => Ok(mode.process_path(entry_path.file_name(), vm)),
                        Err(err) => Err(OSErrorBuilder::with_filename(&err, path.clone(), vm)),
                    })
                    .collect::<PyResult<_>>()?
            }
            OsPathOrFd::Fd(fno) => {
                #[cfg(not(all(unix, not(target_os = "redox"))))]
                {
                    let _ = fno;
                    return Err(
                        vm.new_not_implemented_error("can't pass fd to listdir on this platform")
                    );
                }
                #[cfg(all(unix, not(target_os = "redox")))]
                {
                    use rustpython_common::os::ffi::OsStrExt;
                    let new_fd = nix::unistd::dup(fno).map_err(|e| e.into_pyexception(vm))?;
                    let mut dir =
                        nix::dir::Dir::from_fd(new_fd).map_err(|e| e.into_pyexception(vm))?;
                    dir.iter()
                        .filter_map_ok(|entry| {
                            let fname = entry.file_name().to_bytes();
                            match fname {
                                b"." | b".." => None,
                                _ => Some(
                                    OutputMode::String
                                        .process_path(std::ffi::OsStr::from_bytes(fname), vm),
                                ),
                            }
                        })
                        .collect::<Result<_, _>>()
                        .map_err(|e| e.into_pyexception(vm))?
                }
            }
        };
        Ok(list)
    }

    #[cfg(not(windows))]
    fn env_bytes_as_bytes(obj: &crate::function::Either<PyStrRef, PyBytesRef>) -> &[u8] {
        match obj {
            crate::function::Either::A(s) => s.as_bytes(),
            crate::function::Either::B(b) => b.as_bytes(),
        }
    }

    #[cfg(windows)]
    unsafe extern "C" {
        fn _wputenv(envstring: *const u16) -> libc::c_int;
    }

    /// Check if wide string length exceeds Windows environment variable limit.
    #[cfg(windows)]
    fn check_env_var_len(wide_len: usize, vm: &VirtualMachine) -> PyResult<()> {
        use crate::common::windows::_MAX_ENV;
        if wide_len > _MAX_ENV + 1 {
            return Err(vm.new_value_error(format!(
                "the environment variable is longer than {_MAX_ENV} characters",
            )));
        }
        Ok(())
    }

    #[cfg(windows)]
    #[pyfunction]
    fn putenv(key: PyStrRef, value: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let key_str = key.as_str();
        let value_str = value.as_str();
        // Search from index 1 because on Windows starting '=' is allowed for
        // defining hidden environment variables.
        if key_str.is_empty()
            || key_str.get(1..).is_some_and(|s| s.contains('='))
            || key_str.contains('\0')
            || value_str.contains('\0')
        {
            return Err(vm.new_value_error("illegal environment variable name"));
        }
        let env_str = format!("{}={}", key_str, value_str);
        let wide = env_str.to_wide_with_nul();
        check_env_var_len(wide.len(), vm)?;

        // Use _wputenv like CPython (not SetEnvironmentVariableW) to update CRT environ
        let result = unsafe { suppress_iph!(_wputenv(wide.as_ptr())) };
        if result != 0 {
            return Err(vm.new_last_errno_error());
        }
        Ok(())
    }

    #[cfg(not(windows))]
    #[pyfunction]
    fn putenv(
        key: crate::function::Either<PyStrRef, PyBytesRef>,
        value: crate::function::Either<PyStrRef, PyBytesRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let key = env_bytes_as_bytes(&key);
        let value = env_bytes_as_bytes(&value);
        if key.contains(&b'\0') || value.contains(&b'\0') {
            return Err(vm.new_value_error("embedded null byte"));
        }
        if key.is_empty() || key.contains(&b'=') {
            return Err(vm.new_value_error("illegal environment variable name"));
        }
        let key = super::bytes_as_os_str(key, vm)?;
        let value = super::bytes_as_os_str(value, vm)?;
        // SAFETY: requirements forwarded from the caller
        unsafe { env::set_var(key, value) };
        Ok(())
    }

    #[cfg(windows)]
    #[pyfunction]
    fn unsetenv(key: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let key_str = key.as_str();
        // Search from index 1 because on Windows starting '=' is allowed for
        // defining hidden environment variables.
        if key_str.is_empty()
            || key_str.get(1..).is_some_and(|s| s.contains('='))
            || key_str.contains('\0')
        {
            return Err(vm.new_value_error("illegal environment variable name"));
        }
        // "key=" to unset (empty value removes the variable)
        let env_str = format!("{}=", key_str);
        let wide = env_str.to_wide_with_nul();
        check_env_var_len(wide.len(), vm)?;

        // Use _wputenv like CPython (not SetEnvironmentVariableW) to update CRT environ
        let result = unsafe { suppress_iph!(_wputenv(wide.as_ptr())) };
        if result != 0 {
            return Err(vm.new_last_errno_error());
        }
        Ok(())
    }

    #[cfg(not(windows))]
    #[pyfunction]
    fn unsetenv(
        key: crate::function::Either<PyStrRef, PyBytesRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let key = env_bytes_as_bytes(&key);
        if key.contains(&b'\0') {
            return Err(vm.new_value_error("embedded null byte"));
        }
        if key.is_empty() || key.contains(&b'=') {
            let x = vm.new_errno_error(
                22,
                format!(
                    "Invalid argument: {}",
                    core::str::from_utf8(key).unwrap_or("<bytes encoding failure>")
                ),
            );

            return Err(x.upcast());
        }
        let key = super::bytes_as_os_str(key, vm)?;
        // SAFETY: requirements forwarded from the caller
        unsafe { env::remove_var(key) };
        Ok(())
    }

    #[pyfunction]
    fn readlink(path: OsPath, dir_fd: DirFd<'_, 0>, vm: &VirtualMachine) -> PyResult {
        let mode = path.mode();
        let [] = dir_fd.0;
        let path =
            fs::read_link(&path).map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))?;
        Ok(mode.process_path(path, vm))
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
        #[cfg(windows)]
        ino: AtomicCell<Option<u128>>,
        #[cfg(not(any(unix, windows)))]
        ino: AtomicCell<Option<u64>>,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION), with(Representable))]
    impl DirEntry {
        #[pygetset]
        fn name(&self, vm: &VirtualMachine) -> PyResult {
            Ok(self.mode.process_path(&self.file_name, vm))
        }

        #[pygetset]
        fn path(&self, vm: &VirtualMachine) -> PyResult {
            Ok(self.mode.process_path(&self.pathval, vm))
        }

        #[pymethod]
        fn is_dir(&self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<bool> {
            // Use cached file_type first to avoid stat() calls that may fail
            if let Ok(file_type) = &self.file_type
                && (!follow_symlinks.0 || !file_type.is_symlink())
            {
                return Ok(file_type.is_dir());
            }
            match super::fs_metadata(&self.pathval, follow_symlinks.0) {
                Ok(meta) => Ok(meta.is_dir()),
                Err(e) => {
                    if e.kind() == io::ErrorKind::NotFound {
                        Ok(false)
                    } else {
                        Err(e.into_pyexception(vm))
                    }
                }
            }
        }

        #[pymethod]
        fn is_file(&self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<bool> {
            // Use cached file_type first to avoid stat() calls that may fail
            if let Ok(file_type) = &self.file_type
                && (!follow_symlinks.0 || !file_type.is_symlink())
            {
                return Ok(file_type.is_file());
            }
            match super::fs_metadata(&self.pathval, follow_symlinks.0) {
                Ok(meta) => Ok(meta.is_file()),
                Err(e) => {
                    if e.kind() == io::ErrorKind::NotFound {
                        Ok(false)
                    } else {
                        Err(e.into_pyexception(vm))
                    }
                }
            }
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
            dir_fd: DirFd<'_, { STAT_DIR_FD as usize }>,
            follow_symlinks: FollowSymlinks,
            vm: &VirtualMachine,
        ) -> PyResult {
            let do_stat = |follow_symlinks| {
                stat(
                    OsPath {
                        path: self.pathval.as_os_str().to_owned(),
                        origin: None,
                    }
                    .into(),
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
                        lstat().cloned()
                    }
                })?
            } else {
                lstat()?
            };
            Ok(stat.clone())
        }

        #[cfg(windows)]
        #[pymethod]
        fn inode(&self, vm: &VirtualMachine) -> PyResult<u128> {
            match self.ino.load() {
                Some(ino) => Ok(ino),
                None => {
                    let stat = stat_inner(
                        OsPath::new_str(self.pathval.as_os_str()).into(),
                        DirFd::default(),
                        FollowSymlinks(false),
                    )
                    .map_err(|e| e.into_pyexception(vm))?
                    .ok_or_else(|| crate::exceptions::cstring_error(vm))?;
                    // On Windows, combine st_ino and st_ino_high into 128-bit value
                    #[cfg(windows)]
                    let ino: u128 = stat.st_ino as u128 | ((stat.st_ino_high as u128) << 64);
                    #[cfg(not(windows))]
                    let ino: u128 = stat.st_ino as u128;
                    // Err(T) means other thread set `ino` at the mean time which is safe to ignore
                    let _ = self.ino.compare_exchange(None, Some(ino));
                    Ok(ino)
                }
            }
        }

        #[cfg(unix)]
        #[pymethod]
        fn inode(&self, _vm: &VirtualMachine) -> PyResult<u64> {
            Ok(self.ino.load())
        }

        #[cfg(not(any(unix, windows)))]
        #[pymethod]
        fn inode(&self, _vm: &VirtualMachine) -> PyResult<Option<u64>> {
            Ok(self.ino.load())
        }

        #[cfg(not(windows))]
        #[pymethod]
        const fn is_junction(&self, _vm: &VirtualMachine) -> PyResult<bool> {
            Ok(false)
        }

        #[cfg(windows)]
        #[pymethod]
        fn is_junction(&self, _vm: &VirtualMachine) -> PyResult<bool> {
            Ok(junction::exists(self.pathval.clone()).unwrap_or(false))
        }

        #[pymethod]
        fn __fspath__(&self, vm: &VirtualMachine) -> PyResult {
            self.path(vm)
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyGenericAlias {
            PyGenericAlias::from_args(cls, args, vm)
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot pickle 'DirEntry' object".to_owned()))
        }
    }

    impl Representable for DirEntry {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let name = match zelf.as_object().get_attr("name", vm) {
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
                if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
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
    }
    #[pyattr]
    #[pyclass(name = "ScandirIter")]
    #[derive(Debug, PyPayload)]
    struct ScandirIterator {
        entries: PyRwLock<Option<fs::ReadDir>>,
        mode: OutputMode,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION), with(IterNext, Iterable))]
    impl ScandirIterator {
        #[pymethod]
        fn close(&self) {
            let entryref: &mut Option<fs::ReadDir> = &mut self.entries.write();
            let _dropped = entryref.take();
        }

        #[pymethod]
        const fn __enter__(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod]
        fn __exit__(zelf: PyRef<Self>, _args: FuncArgs) {
            zelf.close()
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot pickle 'ScandirIterator' object".to_owned()))
        }
    }
    impl SelfIter for ScandirIterator {}
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
                            // TODO: wasi is nightly
                            // #[cfg(target_os = "wasi")]
                            // let ino = {
                            //     use std::os::wasi::fs::DirEntryExt;
                            //     entry.ino()
                            // };
                            #[cfg(not(unix))]
                            let ino = None;

                            let pathval = entry.path();

                            // On Windows, pre-cache lstat from directory entry metadata
                            // This allows stat() to return cached data even if file is removed
                            #[cfg(windows)]
                            let lstat = {
                                let cell = OnceCell::new();
                                if let Ok(stat_struct) =
                                    crate::windows::win32_xstat(pathval.as_os_str(), false)
                                {
                                    let stat_obj =
                                        StatResultData::from_stat(&stat_struct, vm).to_pyobject(vm);
                                    let _ = cell.set(stat_obj);
                                }
                                cell
                            };
                            #[cfg(not(windows))]
                            let lstat = OnceCell::new();

                            Ok(PyIterReturn::Return(
                                DirEntry {
                                    file_name: entry.file_name(),
                                    pathval,
                                    file_type: entry.file_type(),
                                    mode: zelf.mode,
                                    lstat,
                                    stat: OnceCell::new(),
                                    ino: AtomicCell::new(ino),
                                }
                                .into_ref(&vm.ctx)
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
    fn scandir(path: OptionalArg<OsPath>, vm: &VirtualMachine) -> PyResult {
        let path = path.unwrap_or_else(|| OsPath::new_str("."));
        let entries = fs::read_dir(&path.path)
            .map_err(|err| OSErrorBuilder::with_filename(&err, path.clone(), vm))?;
        Ok(ScandirIterator {
            entries: PyRwLock::new(Some(entries)),
            mode: path.mode(),
        }
        .into_ref(&vm.ctx)
        .into())
    }

    #[derive(Debug, FromArgs)]
    #[pystruct_sequence_data]
    struct StatResultData {
        pub st_mode: PyIntRef,
        pub st_ino: PyIntRef,
        pub st_dev: PyIntRef,
        pub st_nlink: PyIntRef,
        pub st_uid: PyIntRef,
        pub st_gid: PyIntRef,
        pub st_size: PyIntRef,
        // Indices 7-9: integer seconds
        #[pyarg(positional, default)]
        #[pystruct_sequence(unnamed)]
        pub st_atime_int: libc::time_t,
        #[pyarg(positional, default)]
        #[pystruct_sequence(unnamed)]
        pub st_mtime_int: libc::time_t,
        #[pyarg(positional, default)]
        #[pystruct_sequence(unnamed)]
        pub st_ctime_int: libc::time_t,
        // Float time attributes
        #[pyarg(any, default)]
        #[pystruct_sequence(skip)]
        pub st_atime: f64,
        #[pyarg(any, default)]
        #[pystruct_sequence(skip)]
        pub st_mtime: f64,
        #[pyarg(any, default)]
        #[pystruct_sequence(skip)]
        pub st_ctime: f64,
        // Nanosecond attributes
        #[pyarg(any, default)]
        #[pystruct_sequence(skip)]
        pub st_atime_ns: i128,
        #[pyarg(any, default)]
        #[pystruct_sequence(skip)]
        pub st_mtime_ns: i128,
        #[pyarg(any, default)]
        #[pystruct_sequence(skip)]
        pub st_ctime_ns: i128,
        // Unix-specific attributes
        #[cfg(not(windows))]
        #[pyarg(any, default)]
        #[pystruct_sequence(skip)]
        pub st_blksize: i64,
        #[cfg(not(windows))]
        #[pyarg(any, default)]
        #[pystruct_sequence(skip)]
        pub st_blocks: i64,
        #[cfg(windows)]
        #[pyarg(any, default)]
        #[pystruct_sequence(skip)]
        pub st_reparse_tag: u32,
        #[cfg(windows)]
        #[pyarg(any, default)]
        #[pystruct_sequence(skip)]
        pub st_file_attributes: u32,
    }

    impl StatResultData {
        fn from_stat(stat: &StatStruct, vm: &VirtualMachine) -> Self {
            let (atime, mtime, ctime);
            #[cfg(any(unix, windows))]
            #[cfg(not(any(target_os = "netbsd", target_os = "wasi")))]
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

            #[cfg(windows)]
            let st_reparse_tag = stat.st_reparse_tag;
            #[cfg(windows)]
            let st_file_attributes = stat.st_file_attributes;

            // On Windows, combine st_ino and st_ino_high into a 128-bit value
            // like _pystat_l128_from_l64_l64
            #[cfg(windows)]
            let st_ino: u128 = stat.st_ino as u128 | ((stat.st_ino_high as u128) << 64);
            #[cfg(not(windows))]
            let st_ino = stat.st_ino;

            #[cfg(not(windows))]
            #[allow(clippy::useless_conversion)] // needed for 32-bit platforms
            let st_blksize = i64::from(stat.st_blksize);
            #[cfg(not(windows))]
            #[allow(clippy::useless_conversion)] // needed for 32-bit platforms
            let st_blocks = i64::from(stat.st_blocks);

            Self {
                st_mode: vm.ctx.new_pyref(stat.st_mode),
                st_ino: vm.ctx.new_pyref(st_ino),
                st_dev: vm.ctx.new_pyref(stat.st_dev),
                st_nlink: vm.ctx.new_pyref(stat.st_nlink),
                st_uid: vm.ctx.new_pyref(stat.st_uid),
                st_gid: vm.ctx.new_pyref(stat.st_gid),
                st_size: vm.ctx.new_pyref(stat.st_size),
                st_atime_int: atime.0,
                st_mtime_int: mtime.0,
                st_ctime_int: ctime.0,
                st_atime: to_f64(atime),
                st_mtime: to_f64(mtime),
                st_ctime: to_f64(ctime),
                st_atime_ns: to_ns(atime),
                st_mtime_ns: to_ns(mtime),
                st_ctime_ns: to_ns(ctime),
                #[cfg(not(windows))]
                st_blksize,
                #[cfg(not(windows))]
                st_blocks,
                #[cfg(windows)]
                st_reparse_tag,
                #[cfg(windows)]
                st_file_attributes,
            }
        }
    }

    #[pyattr]
    #[pystruct_sequence(name = "stat_result", module = "os", data = "StatResultData")]
    struct PyStatResult;

    #[pyclass(with(PyStructSequence))]
    impl PyStatResult {
        #[pyslot]
        fn slot_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let flatten_args = |r: &[PyObjectRef]| {
                let mut vec_args = Vec::from(r);
                loop {
                    if let Ok(obj) = vec_args.iter().exactly_one() {
                        match obj.downcast_ref::<PyTuple>() {
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

            let stat: StatResultData = args.bind(vm)?;
            Ok(stat.to_pyobject(vm))
        }
    }

    #[cfg(windows)]
    fn stat_inner(
        file: OsPathOrFd<'_>,
        dir_fd: DirFd<'_, { STAT_DIR_FD as usize }>,
        follow_symlinks: FollowSymlinks,
    ) -> io::Result<Option<StatStruct>> {
        // TODO: replicate CPython's win32_xstat
        let [] = dir_fd.0;
        match file {
            OsPathOrFd::Path(path) => crate::windows::win32_xstat(&path.path, follow_symlinks.0),
            OsPathOrFd::Fd(fd) => crate::common::fileutils::fstat(fd),
        }
        .map(Some)
    }

    #[cfg(not(windows))]
    fn stat_inner(
        file: OsPathOrFd<'_>,
        dir_fd: DirFd<'_, { STAT_DIR_FD as usize }>,
        follow_symlinks: FollowSymlinks,
    ) -> io::Result<Option<StatStruct>> {
        let mut stat = core::mem::MaybeUninit::uninit();
        let ret = match file {
            OsPathOrFd::Path(path) => {
                use rustpython_common::os::ffi::OsStrExt;
                let path = path.as_ref().as_os_str().as_bytes();
                let path = match alloc::ffi::CString::new(path) {
                    Ok(x) => x,
                    Err(_) => return Ok(None),
                };

                #[cfg(not(target_os = "redox"))]
                let fstatat_ret = dir_fd.raw_opt().map(|dir_fd| {
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
            OsPathOrFd::Fd(fd) => unsafe { libc::fstat(fd.as_raw(), stat.as_mut_ptr()) },
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(unsafe { stat.assume_init() }))
    }

    #[pyfunction]
    #[pyfunction(name = "fstat")]
    fn stat(
        file: OsPathOrFd<'_>,
        dir_fd: DirFd<'_, { STAT_DIR_FD as usize }>,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult {
        let stat = stat_inner(file.clone(), dir_fd, follow_symlinks)
            .map_err(|err| OSErrorBuilder::with_filename(&err, file, vm))?
            .ok_or_else(|| crate::exceptions::cstring_error(vm))?;
        Ok(StatResultData::from_stat(&stat, vm).to_pyobject(vm))
    }

    #[pyfunction]
    fn lstat(
        file: OsPath,
        dir_fd: DirFd<'_, { STAT_DIR_FD as usize }>,
        vm: &VirtualMachine,
    ) -> PyResult {
        stat(file.into(), dir_fd, FollowSymlinks(false), vm)
    }

    fn curdir_inner(vm: &VirtualMachine) -> PyResult<PathBuf> {
        env::current_dir().map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn getcwd(vm: &VirtualMachine) -> PyResult {
        Ok(OutputMode::String.process_path(curdir_inner(vm)?, vm))
    }

    #[pyfunction]
    fn getcwdb(vm: &VirtualMachine) -> PyResult {
        Ok(OutputMode::Bytes.process_path(curdir_inner(vm)?, vm))
    }

    #[pyfunction]
    fn chdir(path: OsPath, vm: &VirtualMachine) -> PyResult<()> {
        env::set_current_dir(&path.path)
            .map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))?;

        #[cfg(windows)]
        {
            // win32_wchdir()

            // On Windows, set the per-drive CWD environment variable (=X:)
            // This is required for GetFullPathNameW to work correctly with drive-relative paths

            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::System::Environment::SetEnvironmentVariableW;

            if let Ok(cwd) = env::current_dir() {
                let cwd_str = cwd.as_os_str();
                let mut cwd_wide: Vec<u16> = cwd_str.encode_wide().collect();

                // Check for UNC-like paths (\\server\share or //server/share)
                // wcsncmp(new_path, L"\\\\", 2) == 0 || wcsncmp(new_path, L"//", 2) == 0
                let is_unc_like_path = cwd_wide.len() >= 2
                    && ((cwd_wide[0] == b'\\' as u16 && cwd_wide[1] == b'\\' as u16)
                        || (cwd_wide[0] == b'/' as u16 && cwd_wide[1] == b'/' as u16));

                if !is_unc_like_path {
                    // Create env var name "=X:" where X is the drive letter
                    let env_name: [u16; 4] = [b'=' as u16, cwd_wide[0], b':' as u16, 0];
                    cwd_wide.push(0); // null-terminate the path
                    unsafe {
                        SetEnvironmentVariableW(env_name.as_ptr(), cwd_wide.as_ptr());
                    }
                }
            }
        }

        Ok(())
    }

    #[pyfunction]
    fn fspath(path: PyObjectRef, vm: &VirtualMachine) -> PyResult<FsPath> {
        FsPath::try_from_path_like(path, false, vm)
    }

    #[pyfunction]
    #[pyfunction(name = "replace")]
    fn rename(src: PyObjectRef, dst: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let src = PathConverter::new()
            .function("rename")
            .argument("src")
            .try_path(src, vm)?;
        let dst = PathConverter::new()
            .function("rename")
            .argument("dst")
            .try_path(dst, vm)?;

        fs::rename(&src.path, &dst.path).map_err(|err| {
            let builder = err.to_os_error_builder(vm);
            let builder = builder.filename(src.filename(vm));
            let builder = builder.filename2(dst.filename(vm));
            builder.build(vm).upcast()
        })
    }

    #[pyfunction]
    fn getpid(vm: &VirtualMachine) -> PyObjectRef {
        let pid = if cfg!(target_arch = "wasm32") {
            // Return an arbitrary value, greater than 1 which is special.
            // The value 42 is picked from wasi-libc
            // https://github.com/WebAssembly/wasi-libc/blob/wasi-sdk-21/libc-bottom-half/getpid/getpid.c
            42
        } else {
            std::process::id()
        };
        vm.ctx.new_int(pid).into()
    }

    #[pyfunction]
    fn cpu_count(vm: &VirtualMachine) -> PyObjectRef {
        let cpu_count = num_cpus::get();
        vm.ctx.new_int(cpu_count).into()
    }

    #[pyfunction]
    fn _exit(code: i32) {
        std::process::exit(code)
    }

    #[pyfunction]
    fn abort() {
        unsafe extern "C" {
            fn abort();
        }
        unsafe { abort() }
    }

    #[pyfunction]
    fn urandom(size: isize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        if size < 0 {
            return Err(vm.new_value_error("negative argument not allowed"));
        }
        let mut buf = vec![0u8; size as usize];
        getrandom::fill(&mut buf).map_err(|e| io::Error::from(e).into_pyexception(vm))?;
        Ok(buf)
    }

    #[pyfunction]
    pub fn isatty(fd: i32) -> bool {
        unsafe { suppress_iph!(libc::isatty(fd)) != 0 }
    }

    #[pyfunction]
    pub fn lseek(
        fd: crt_fd::Borrowed<'_>,
        position: crt_fd::Offset,
        how: i32,
        vm: &VirtualMachine,
    ) -> PyResult<crt_fd::Offset> {
        #[cfg(not(windows))]
        let res = unsafe { suppress_iph!(libc::lseek(fd.as_raw(), position, how)) };
        #[cfg(windows)]
        let res = unsafe {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem;
            let handle = crt_fd::as_handle(fd).map_err(|e| e.into_pyexception(vm))?;
            let mut distance_to_move: [i32; 2] = core::mem::transmute(position);
            let ret = FileSystem::SetFilePointer(
                handle.as_raw_handle(),
                distance_to_move[0],
                &mut distance_to_move[1],
                how as _,
            );
            if ret == FileSystem::INVALID_SET_FILE_POINTER {
                -1
            } else {
                distance_to_move[0] = ret as _;
                core::mem::transmute::<[i32; 2], i64>(distance_to_move)
            }
        };
        if res < 0 {
            Err(vm.new_last_os_error())
        } else {
            Ok(res)
        }
    }

    #[pyfunction]
    fn link(
        src: OsPath,
        dst: OsPath,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let src_cstr = std::ffi::CString::new(src.path.as_os_str().as_bytes())
                .map_err(|_| vm.new_value_error("embedded null byte".to_owned()))?;
            let dst_cstr = std::ffi::CString::new(dst.path.as_os_str().as_bytes())
                .map_err(|_| vm.new_value_error("embedded null byte".to_owned()))?;

            let flags = if follow_symlinks.0 {
                libc::AT_SYMLINK_FOLLOW
            } else {
                0
            };

            let ret = unsafe {
                libc::linkat(
                    libc::AT_FDCWD,
                    src_cstr.as_ptr(),
                    libc::AT_FDCWD,
                    dst_cstr.as_ptr(),
                    flags,
                )
            };

            if ret != 0 {
                let err = std::io::Error::last_os_error();
                let builder = err.to_os_error_builder(vm);
                let builder = builder.filename(src.filename(vm));
                let builder = builder.filename2(dst.filename(vm));
                return Err(builder.build(vm).upcast());
            }

            Ok(())
        }

        #[cfg(not(unix))]
        {
            // On non-Unix platforms, ignore follow_symlinks if it's the default value
            // or raise NotImplementedError if explicitly set to False
            if !follow_symlinks.0 {
                return Err(vm.new_not_implemented_error(
                    "link: follow_symlinks unavailable on this platform".to_owned(),
                ));
            }

            fs::hard_link(&src.path, &dst.path).map_err(|err| {
                let builder = err.to_os_error_builder(vm);
                let builder = builder.filename(src.filename(vm));
                let builder = builder.filename2(dst.filename(vm));
                builder.build(vm).upcast()
            })
        }
    }

    #[cfg(any(unix, windows))]
    #[pyfunction]
    fn system(command: PyStrRef, vm: &VirtualMachine) -> PyResult<i32> {
        let cstr = command.to_cstring(vm)?;
        let x = unsafe { libc::system(cstr.as_ptr()) };
        Ok(x)
    }

    #[derive(FromArgs)]
    struct UtimeArgs<'fd> {
        path: OsPath,
        #[pyarg(any, default)]
        times: Option<PyTupleRef>,
        #[pyarg(named, default)]
        ns: Option<PyTupleRef>,
        #[pyarg(flatten)]
        dir_fd: DirFd<'fd, { UTIME_DIR_FD as usize }>,
        #[pyarg(flatten)]
        follow_symlinks: FollowSymlinks,
    }

    #[pyfunction]
    fn utime(args: UtimeArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        let parse_tup = |tup: &Py<PyTuple>| -> Option<(PyObjectRef, PyObjectRef)> {
            if tup.len() != 2 {
                None
            } else {
                Some((tup[0].clone(), tup[1].clone()))
            }
        };
        let (acc, modif) = match (args.times, args.ns) {
            (Some(t), None) => {
                let (a, m) = parse_tup(&t).ok_or_else(|| {
                    vm.new_type_error("utime: 'times' must be either a tuple of two ints or None")
                })?;
                (a.try_into_value(vm)?, m.try_into_value(vm)?)
            }
            (None, Some(ns)) => {
                let (a, m) = parse_tup(&ns)
                    .ok_or_else(|| vm.new_type_error("utime: 'ns' must be a tuple of two ints"))?;
                let ns_in_sec: PyObjectRef = vm.ctx.new_int(1_000_000_000).into();
                let ns_to_dur = |obj: PyObjectRef| {
                    let divmod = vm._divmod(&obj, &ns_in_sec)?;
                    let (div, rem) = divmod
                        .downcast_ref::<PyTuple>()
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
                    "utime: you may specify either 'times' or 'ns' but not both",
                ));
            }
        };
        utime_impl(args.path, acc, modif, args.dir_fd, args.follow_symlinks, vm)
    }

    fn utime_impl(
        path: OsPath,
        acc: Duration,
        modif: Duration,
        dir_fd: DirFd<'_, { UTIME_DIR_FD as usize }>,
        _follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        #[cfg(any(target_os = "wasi", unix))]
        {
            #[cfg(not(target_os = "redox"))]
            {
                let path_for_err = path.clone();
                let path = path.into_cstring(vm)?;

                let ts = |d: Duration| libc::timespec {
                    tv_sec: d.as_secs() as _,
                    tv_nsec: d.subsec_nanos() as _,
                };
                let times = [ts(acc), ts(modif)];

                let ret = unsafe {
                    libc::utimensat(
                        dir_fd.get().as_raw(),
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
                    Err(OSErrorBuilder::with_filename(
                        &io::Error::last_os_error(),
                        path_for_err,
                        vm,
                    ))
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
            type DWORD = u32;
            use windows_sys::Win32::{Foundation::FILETIME, Storage::FileSystem};

            let [] = dir_fd.0;

            if !_follow_symlinks.0 {
                return Err(vm.new_not_implemented_error(
                    "utime: follow_symlinks unavailable on this platform",
                ));
            }

            let ft = |d: Duration| {
                let intervals = ((d.as_secs() as i64 + 11644473600) * 10_000_000)
                    + (d.subsec_nanos() as i64 / 100);
                FILETIME {
                    dwLowDateTime: intervals as DWORD,
                    dwHighDateTime: (intervals >> 32) as DWORD,
                }
            };

            let acc = ft(acc);
            let modif = ft(modif);

            let f = OpenOptions::new()
                .write(true)
                .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS)
                .open(&path)
                .map_err(|err| OSErrorBuilder::with_filename(&err, path.clone(), vm))?;

            let ret = unsafe {
                FileSystem::SetFileTime(f.as_raw_handle() as _, core::ptr::null(), &acc, &modif)
            };

            if ret == 0 {
                Err(OSErrorBuilder::with_filename(
                    &io::Error::last_os_error(),
                    path,
                    vm,
                ))
            } else {
                Ok(())
            }
        }
    }

    #[cfg(all(any(unix, windows), not(target_os = "redox")))]
    #[derive(Debug)]
    #[pystruct_sequence_data]
    struct TimesResultData {
        pub user: f64,
        pub system: f64,
        pub children_user: f64,
        pub children_system: f64,
        pub elapsed: f64,
    }

    #[cfg(all(any(unix, windows), not(target_os = "redox")))]
    #[pyattr]
    #[pystruct_sequence(name = "times_result", module = "os", data = "TimesResultData")]
    struct PyTimesResult;

    #[cfg(all(any(unix, windows), not(target_os = "redox")))]
    #[pyclass(with(PyStructSequence))]
    impl PyTimesResult {}

    #[cfg(all(any(unix, windows), not(target_os = "redox")))]
    #[pyfunction]
    fn times(vm: &VirtualMachine) -> PyResult {
        #[cfg(windows)]
        {
            use std::mem::MaybeUninit;
            use windows_sys::Win32::{Foundation::FILETIME, System::Threading};

            let mut _create = MaybeUninit::<FILETIME>::uninit();
            let mut _exit = MaybeUninit::<FILETIME>::uninit();
            let mut kernel = MaybeUninit::<FILETIME>::uninit();
            let mut user = MaybeUninit::<FILETIME>::uninit();

            unsafe {
                let h_proc = Threading::GetCurrentProcess();
                Threading::GetProcessTimes(
                    h_proc,
                    _create.as_mut_ptr(),
                    _exit.as_mut_ptr(),
                    kernel.as_mut_ptr(),
                    user.as_mut_ptr(),
                );
            }

            let kernel = unsafe { kernel.assume_init() };
            let user = unsafe { user.assume_init() };

            let times_result = TimesResultData {
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

            let times_result = TimesResultData {
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
    struct CopyFileRangeArgs<'fd> {
        #[pyarg(positional)]
        src: crt_fd::Borrowed<'fd>,
        #[pyarg(positional)]
        dst: crt_fd::Borrowed<'fd>,
        #[pyarg(positional)]
        count: i64,
        #[pyarg(any, default)]
        offset_src: Option<crt_fd::Offset>,
        #[pyarg(any, default)]
        offset_dst: Option<crt_fd::Offset>,
    }

    #[cfg(target_os = "linux")]
    #[pyfunction]
    fn copy_file_range(args: CopyFileRangeArgs<'_>, vm: &VirtualMachine) -> PyResult<usize> {
        #[allow(clippy::unnecessary_option_map_or_else)]
        let p_offset_src = args.offset_src.as_ref().map_or_else(core::ptr::null, |x| x);
        #[allow(clippy::unnecessary_option_map_or_else)]
        let p_offset_dst = args.offset_dst.as_ref().map_or_else(core::ptr::null, |x| x);
        let count: usize = args
            .count
            .try_into()
            .map_err(|_| vm.new_value_error("count should >= 0"))?;

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

        usize::try_from(ret).map_err(|_| vm.new_last_errno_error())
    }

    #[pyfunction]
    fn strerror(e: i32) -> String {
        unsafe { core::ffi::CStr::from_ptr(libc::strerror(e)) }
            .to_string_lossy()
            .into_owned()
    }

    #[pyfunction]
    pub fn ftruncate(fd: crt_fd::Borrowed<'_>, length: crt_fd::Offset) -> io::Result<()> {
        crt_fd::ftruncate(fd, length)
    }

    #[pyfunction]
    fn truncate(path: PyObjectRef, length: crt_fd::Offset, vm: &VirtualMachine) -> PyResult<()> {
        if let Ok(fd) = path.clone().try_into_value(vm) {
            return ftruncate(fd, length).map_err(|e| e.into_pyexception(vm));
        }

        #[cold]
        fn error(
            vm: &VirtualMachine,
            error: std::io::Error,
            path: OsPath,
        ) -> crate::builtins::PyBaseExceptionRef {
            OSErrorBuilder::with_filename(&error, path, vm)
        }

        let path = OsPath::try_from_object(vm, path)?;
        // TODO: just call libc::truncate() on POSIX
        let f = match OpenOptions::new().write(true).open(&path) {
            Ok(f) => f,
            Err(e) => return Err(error(vm, e, path)),
        };
        f.set_len(length as u64).map_err(|e| error(vm, e, path))?;
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

    #[cfg(unix)]
    #[pyfunction]
    fn waitstatus_to_exitcode(status: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let status = u32::try_from(status)
            .map_err(|_| vm.new_value_error(format!("invalid WEXITSTATUS: {status}")))?;

        let status = status as libc::c_int;
        if libc::WIFEXITED(status) {
            return Ok(libc::WEXITSTATUS(status));
        }

        if libc::WIFSIGNALED(status) {
            return Ok(-libc::WTERMSIG(status));
        }

        Err(vm.new_value_error(format!("Invalid wait status: {status}")))
    }

    #[cfg(windows)]
    #[pyfunction]
    fn waitstatus_to_exitcode(status: u64, vm: &VirtualMachine) -> PyResult<u32> {
        let exitcode = status >> 8;
        // ExitProcess() accepts an UINT type:
        // reject exit code which doesn't fit in an UINT
        u32::try_from(exitcode)
            .map_err(|_| vm.new_value_error(format!("Invalid exit code: {exitcode}")))
    }

    #[pyfunction]
    fn device_encoding(fd: i32, _vm: &VirtualMachine) -> PyResult<Option<String>> {
        if !isatty(fd) {
            return Ok(None);
        }

        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "android", target_os = "redox"))] {
                Ok(Some("UTF-8".to_owned()))
            } else if #[cfg(windows)] {
                use windows_sys::Win32::System::Console;
                let cp = match fd {
                    0 => unsafe { Console::GetConsoleCP() },
                    1 | 2 => unsafe { Console::GetConsoleOutputCP() },
                    _ => 0,
                };

                Ok(Some(format!("cp{cp}")))
            } else {
                let encoding = unsafe {
                    let encoding = libc::nl_langinfo(libc::CODESET);
                    if encoding.is_null() || encoding.read() == '\0' as libc::c_char {
                        "UTF-8".to_owned()
                    } else {
                        core::ffi::CStr::from_ptr(encoding).to_string_lossy().into_owned()
                    }
                };

                Ok(Some(encoding))
            }
        }
    }

    #[pystruct_sequence_data]
    #[allow(dead_code)]
    pub(crate) struct TerminalSizeData {
        pub columns: usize,
        pub lines: usize,
    }

    #[pyattr]
    #[pystruct_sequence(name = "terminal_size", module = "os", data = "TerminalSizeData")]
    pub(crate) struct PyTerminalSize;

    #[pyclass(with(PyStructSequence))]
    impl PyTerminalSize {}

    #[derive(Debug)]
    #[pystruct_sequence_data]
    pub(crate) struct UnameResultData {
        pub sysname: String,
        pub nodename: String,
        pub release: String,
        pub version: String,
        pub machine: String,
    }

    #[pyattr]
    #[pystruct_sequence(name = "uname_result", module = "os", data = "UnameResultData")]
    pub(crate) struct PyUnameResult;

    #[pyclass(with(PyStructSequence))]
    impl PyUnameResult {}

    // statvfs_result: Result from statvfs or fstatvfs.
    // = statvfs_result_fields
    #[cfg(all(unix, not(target_os = "redox")))]
    #[derive(Debug)]
    #[pystruct_sequence_data]
    pub(crate) struct StatvfsResultData {
        pub f_bsize: libc::c_ulong,     // filesystem block size
        pub f_frsize: libc::c_ulong,    // fragment size
        pub f_blocks: libc::fsblkcnt_t, // size of fs in f_frsize units
        pub f_bfree: libc::fsblkcnt_t,  // free blocks
        pub f_bavail: libc::fsblkcnt_t, // free blocks for unprivileged users
        pub f_files: libc::fsfilcnt_t,  // inodes
        pub f_ffree: libc::fsfilcnt_t,  // free inodes
        pub f_favail: libc::fsfilcnt_t, // free inodes for unprivileged users
        pub f_flag: libc::c_ulong,      // mount flags
        pub f_namemax: libc::c_ulong,   // maximum filename length
        #[pystruct_sequence(skip)]
        pub f_fsid: libc::c_ulong, // filesystem ID (not in tuple but accessible as attribute)
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyattr]
    #[pystruct_sequence(name = "statvfs_result", module = "os", data = "StatvfsResultData")]
    pub(crate) struct PyStatvfsResult;

    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyclass(with(PyStructSequence))]
    impl PyStatvfsResult {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let seq: PyObjectRef = args.bind(vm)?;
            crate::types::struct_sequence_new(cls, seq, vm)
        }
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    impl StatvfsResultData {
        fn from_statvfs(st: libc::statvfs) -> Self {
            // f_fsid is a struct on some platforms (e.g., Linux fsid_t) and a scalar on others.
            // We extract raw bytes and interpret as a native-endian integer.
            // Note: The value may differ across architectures due to endianness.
            let f_fsid = {
                let ptr = core::ptr::addr_of!(st.f_fsid) as *const u8;
                let size = core::mem::size_of_val(&st.f_fsid);
                if size >= 8 {
                    let bytes = unsafe { core::slice::from_raw_parts(ptr, 8) };
                    u64::from_ne_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6],
                        bytes[7],
                    ]) as libc::c_ulong
                } else if size >= 4 {
                    let bytes = unsafe { core::slice::from_raw_parts(ptr, 4) };
                    u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as libc::c_ulong
                } else {
                    0
                }
            };

            Self {
                f_bsize: st.f_bsize,
                f_frsize: st.f_frsize,
                f_blocks: st.f_blocks,
                f_bfree: st.f_bfree,
                f_bavail: st.f_bavail,
                f_files: st.f_files,
                f_ffree: st.f_ffree,
                f_favail: st.f_favail,
                f_flag: st.f_flag,
                f_namemax: st.f_namemax,
                f_fsid,
            }
        }
    }

    /// Perform a statvfs system call on the given path.
    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyfunction]
    #[pyfunction(name = "fstatvfs")]
    fn statvfs(path: OsPathOrFd<'_>, vm: &VirtualMachine) -> PyResult {
        let mut st: libc::statvfs = unsafe { core::mem::zeroed() };
        let ret = match &path {
            OsPathOrFd::Path(p) => {
                let cpath = p.clone().into_cstring(vm)?;
                unsafe { libc::statvfs(cpath.as_ptr(), &mut st) }
            }
            OsPathOrFd::Fd(fd) => unsafe { libc::fstatvfs(fd.as_raw(), &mut st) },
        };
        if ret != 0 {
            return Err(OSErrorBuilder::with_filename(
                &io::Error::last_os_error(),
                path,
                vm,
            ));
        }
        Ok(StatvfsResultData::from_statvfs(st).to_pyobject(vm))
    }

    pub(super) fn support_funcs() -> Vec<SupportFunc> {
        let mut supports = super::platform::module::support_funcs();
        supports.extend(vec![
            SupportFunc::new("open", Some(false), Some(OPEN_DIR_FD), Some(false)),
            SupportFunc::new("access", Some(false), Some(false), None),
            SupportFunc::new("chdir", None, Some(false), Some(false)),
            // chflags Some, None Some
            SupportFunc::new("link", Some(false), Some(false), Some(cfg!(unix))),
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
            SupportFunc::new("ftruncate", Some(true), Some(false), Some(false)),
            SupportFunc::new("fsync", Some(true), Some(false), Some(false)),
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
    pub(crate) const fn new(
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

pub fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
    let support_funcs = _os::support_funcs();
    let supports_fd = PySet::default().into_ref(&vm.ctx);
    let supports_dir_fd = PySet::default().into_ref(&vm.ctx);
    let supports_follow_symlinks = PySet::default().into_ref(&vm.ctx);
    for support in support_funcs {
        let func_obj = module.get_attr(support.name, vm)?;
        if support.fd.unwrap_or(false) {
            supports_fd.clone().add(func_obj.clone(), vm)?;
        }
        if support.dir_fd.unwrap_or(false) {
            supports_dir_fd.clone().add(func_obj.clone(), vm)?;
        }
        if support.follow_symlinks.unwrap_or(false) {
            supports_follow_symlinks.clone().add(func_obj, vm)?;
        }
    }

    extend_module!(vm, module, {
        "supports_fd" => supports_fd,
        "supports_dir_fd" => supports_dir_fd,
        "supports_follow_symlinks" => supports_follow_symlinks,
        "error" => vm.ctx.exceptions.os_error.to_owned(),
    });

    Ok(())
}

#[cfg(not(windows))]
use super::posix as platform;

#[cfg(windows)]
use super::nt as platform;

pub(crate) use platform::module::MODULE_NAME;
