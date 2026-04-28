// spell-checker:disable

use crate::{
    AsObject, Py, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyModule, PySet},
    convert::{IntoPyException, ToPyException, ToPyObject},
    function::{ArgumentError, FromArgs, FuncArgs},
    host_env::crt_fd,
};
use std::{io, path::Path};

pub(crate) fn fs_metadata<P: AsRef<Path>>(
    path: P,
    follow_symlink: bool,
) -> io::Result<std::fs::Metadata> {
    if follow_symlink {
        crate::host_env::fs::metadata(path.as_ref())
    } else {
        crate::host_env::fs::symlink_metadata(path.as_ref())
    }
}

#[allow(dead_code)]
#[derive(FromArgs, Default)]
pub struct TargetIsDirectory {
    #[pyarg(any, default = false)]
    pub(crate) target_is_directory: bool,
}

cfg_select! {
    all(any(unix, target_os = "wasi"), not(target_os = "redox")) => {
        use libc::AT_FDCWD;
    }
    _ => {
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
                warn_if_bool_fd(&o, vm).map_err(Into::<ArgumentError>::into)?;
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
    rustpython_host_env::os::bytes_as_os_str(b)
        .map_err(|_| vm.new_unicode_decode_error("can't decode path for utf-8"))
}

pub(crate) fn warn_if_bool_fd(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    use crate::class::StaticType;
    if obj
        .class()
        .is(crate::builtins::bool_::PyBool::static_type())
    {
        crate::stdlib::_warnings::warn(
            vm.ctx.exceptions.runtime_warning,
            "bool is used as a file descriptor".to_owned(),
            1,
            vm,
        )?;
    }
    Ok(())
}

impl TryFromObject for crt_fd::Owned {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        warn_if_bool_fd(&obj, vm)?;
        let fd = crt_fd::Raw::try_from_object(vm, obj)?;
        unsafe { crt_fd::Owned::try_from_raw(fd) }.map_err(|e| e.into_pyexception(vm))
    }
}

impl TryFromObject for crt_fd::Borrowed<'_> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        warn_if_bool_fd(&obj, vm)?;
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
    use crate::host_env::fileutils::StatStruct;
    #[cfg(windows)]
    use crate::host_env::windows::ToWideString;
    #[cfg(any(unix, windows))]
    use crate::utils::ToCString;
    use crate::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
        builtins::{
            PyBytesRef, PyGenericAlias, PyIntRef, PyStrRef, PyTuple, PyTupleRef, PyTypeRef,
        },
        common::lock::{OnceCell, PyRwLock},
        convert::{IntoPyException, ToPyObject},
        exceptions::{OSErrorBuilder, ToOSErrorBuilder},
        function::{ArgBytesLike, ArgMemoryBuffer, FsPath, FuncArgs, OptionalArg},
        host_env::crt_fd,
        ospath::{OsPath, OsPathOrFd, OutputMode, PathConverter},
        protocol::PyIterReturn,
        recursion::ReprGuard,
        types::{Destructor, IterNext, Iterable, PyStructSequence, Representable, SelfIter},
        vm::VirtualMachine,
    };
    use core::time::Duration;
    use crossbeam_utils::atomic::AtomicCell;
    use rustpython_common::wtf8::Wtf8Buf;
    #[cfg(windows)]
    use rustpython_host_env::nt as host_nt;
    #[cfg(all(unix, not(target_os = "redox")))]
    use rustpython_host_env::posix as host_posix;
    use std::{fs, io, path::PathBuf, time::SystemTime};

    const OPEN_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    pub(crate) const MKDIR_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    const STAT_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    const UTIME_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    pub(crate) const SYMLINK_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    pub(crate) const UNLINK_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    const RMDIR_DIR_FD: bool = cfg!(not(any(windows, target_os = "redox")));
    const SCANDIR_FD: bool = cfg!(all(unix, not(target_os = "redox")));

    #[pyattr]
    use libc::{O_APPEND, O_CREAT, O_EXCL, O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY};

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
        fd.map_err(|err| OSErrorBuilder::with_filename_from_errno(&err, name, vm))
    }

    #[pyfunction]
    fn fsync(fd: crt_fd::Borrowed<'_>) -> io::Result<()> {
        crt_fd::fsync(fd)
    }

    #[pyfunction]
    fn read(fd: crt_fd::Borrowed<'_>, n: usize, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let mut buffer = vec![0u8; n];
        loop {
            match vm.allow_threads(|| crt_fd::read(fd, &mut buffer)) {
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
    fn readinto(
        fd: crt_fd::Borrowed<'_>,
        buffer: ArgMemoryBuffer,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        buffer.with_ref(|buf| {
            loop {
                match vm.allow_threads(|| crt_fd::read(fd, buf)) {
                    Ok(n) => return Ok(n),
                    Err(e) if e.raw_os_error() == Some(libc::EINTR) => {
                        vm.check_signals()?;
                        continue;
                    }
                    Err(e) => return Err(e.into_pyexception(vm)),
                }
            }
        })
    }

    #[pyfunction]
    fn write(fd: crt_fd::Borrowed<'_>, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
        let owned = data.with_ref(|b| b.to_vec());
        loop {
            match vm.allow_threads(|| crt_fd::write(fd, &owned)) {
                Ok(n) => return Ok(n),
                Err(e) if e.raw_os_error() == Some(libc::EINTR) => {
                    vm.check_signals()?;
                    continue;
                }
                Err(e) => return Err(e.into_pyexception(vm)),
            }
        }
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
            return if let Err(err) =
                crate::host_env::posix::make_dir_at(fd, c_path.as_c_str(), mode as u32)
            {
                Err(OSErrorBuilder::with_filename(&err, path, vm))
            } else {
                Ok(())
            };
        }
        #[cfg(target_os = "redox")]
        let [] = dir_fd.0;
        if let Err(err) = crate::host_env::posix::make_dir(c_path.as_c_str(), mode as u32) {
            return Err(OSErrorBuilder::with_filename(&err, path, vm));
        }
        Ok(())
    }

    #[pyfunction]
    fn mkdirs(path: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let os_path = vm.fsencode(&path)?;
        crate::host_env::fs::create_dir_all(&*os_path).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(windows))]
    #[pyfunction]
    fn rmdir(
        path: OsPath,
        dir_fd: DirFd<'_, { RMDIR_DIR_FD as usize }>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        #[cfg(not(target_os = "redox"))]
        if let Some(fd) = dir_fd.raw_opt() {
            let c_path = path.clone().into_cstring(vm)?;
            return if let Err(err) = crate::host_env::posix::remove_dir_at(fd, c_path.as_c_str()) {
                Err(OSErrorBuilder::with_filename(&err, path, vm))
            } else {
                Ok(())
            };
        }
        #[cfg(target_os = "redox")]
        let [] = dir_fd.0;
        crate::host_env::fs::remove_dir(&path)
            .map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))
    }

    #[cfg(windows)]
    #[pyfunction]
    fn rmdir(path: OsPath, dir_fd: DirFd<'_, 0>, vm: &VirtualMachine) -> PyResult<()> {
        let [] = dir_fd.0;
        crate::host_env::fs::remove_dir(&path)
            .map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))
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
                let dir_iter = match crate::host_env::fs::read_dir(&path) {
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
                    let mut dir = host_posix::FdDirStream::from_fd(fno.into())
                        .map_err(|e| e.into_pyexception(vm))?;
                    let mut list = Vec::new();
                    while let Some(entry) = dir.next_entry().map_err(|e| e.into_pyexception(vm))? {
                        list.push(
                            OutputMode::String.process_path(
                                rustpython_host_env::os::bytes_as_os_str(&entry.name)
                                    .expect("unix dir entry names are arbitrary bytes"),
                                vm,
                            ),
                        );
                    }
                    list
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
        use crate::host_env::windows::_MAX_ENV;
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
        let key_str = key.expect_str();
        let value_str = value.expect_str();
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
        unsafe { crate::host_env::os::set_var(key, value) };
        Ok(())
    }

    #[cfg(windows)]
    #[pyfunction]
    fn unsetenv(key: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let key_str = key.expect_str();
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
        unsafe { crate::host_env::os::remove_var(key) };
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
        /// dirent d_type value, used when file_type is unavailable (fd-based scandir)
        #[cfg(unix)]
        d_type: Option<u8>,
        /// Parent directory fd for fd-based scandir, used for fstatat
        #[cfg(not(any(windows, target_os = "redox")))]
        dir_fd: Option<crt_fd::Raw>,
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

        /// Build the DirFd to use for stat calls.
        /// If this entry was produced by fd-based scandir, use the stored dir_fd
        /// so that fstatat(dir_fd, name, ...) is used instead of stat(full_path).
        fn stat_dir_fd(&self) -> DirFd<'_, { STAT_DIR_FD as usize }> {
            #[cfg(not(any(windows, target_os = "redox")))]
            if let Some(raw_fd) = self.dir_fd {
                // Safety: the fd came from os.open() and is borrowed for
                // the lifetime of this DirEntry reference.
                let borrowed = unsafe { crt_fd::Borrowed::borrow_raw(raw_fd) };
                return DirFd([borrowed; STAT_DIR_FD as usize]);
            }
            DirFd::default()
        }

        /// Stat-based mode test fallback. Uses fstatat when dir_fd is available.
        #[cfg(unix)]
        fn test_mode_via_stat(
            &self,
            follow_symlinks: bool,
            mode_bits: u32,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            match self.stat(self.stat_dir_fd(), FollowSymlinks(follow_symlinks), vm) {
                Ok(stat_obj) => {
                    let st_mode: i32 = stat_obj.get_attr("st_mode", vm)?.try_into_value(vm)?;
                    #[allow(clippy::unnecessary_cast)]
                    Ok((st_mode as u32 & libc::S_IFMT as u32) == mode_bits)
                }
                Err(e) => {
                    if e.fast_isinstance(vm.ctx.exceptions.file_not_found_error) {
                        Ok(false)
                    } else {
                        Err(e)
                    }
                }
            }
        }

        #[pymethod]
        fn is_dir(&self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<bool> {
            if let Ok(file_type) = &self.file_type
                && (!follow_symlinks.0 || !file_type.is_symlink())
            {
                return Ok(file_type.is_dir());
            }
            #[cfg(unix)]
            if let Some(dt) = self.d_type {
                let is_symlink = dt == libc::DT_LNK;
                let need_stat = dt == libc::DT_UNKNOWN || (follow_symlinks.0 && is_symlink);
                if !need_stat {
                    return Ok(dt == libc::DT_DIR);
                }
            }
            #[cfg(unix)]
            return self.test_mode_via_stat(follow_symlinks.0, libc::S_IFDIR as _, vm);
            #[cfg(not(unix))]
            match super::fs_metadata(&self.pathval, follow_symlinks.0) {
                Ok(meta) => Ok(meta.is_dir()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e.into_pyexception(vm)),
            }
        }

        #[pymethod]
        fn is_file(&self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<bool> {
            if let Ok(file_type) = &self.file_type
                && (!follow_symlinks.0 || !file_type.is_symlink())
            {
                return Ok(file_type.is_file());
            }
            #[cfg(unix)]
            if let Some(dt) = self.d_type {
                let is_symlink = dt == libc::DT_LNK;
                let need_stat = dt == libc::DT_UNKNOWN || (follow_symlinks.0 && is_symlink);
                if !need_stat {
                    return Ok(dt == libc::DT_REG);
                }
            }
            #[cfg(unix)]
            return self.test_mode_via_stat(follow_symlinks.0, libc::S_IFREG as _, vm);
            #[cfg(not(unix))]
            match super::fs_metadata(&self.pathval, follow_symlinks.0) {
                Ok(meta) => Ok(meta.is_file()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e.into_pyexception(vm)),
            }
        }

        #[pymethod]
        fn is_symlink(&self, vm: &VirtualMachine) -> PyResult<bool> {
            if let Ok(file_type) = &self.file_type {
                return Ok(file_type.is_symlink());
            }
            #[cfg(unix)]
            if let Some(dt) = self.d_type
                && dt != libc::DT_UNKNOWN
            {
                return Ok(dt == libc::DT_LNK);
            }
            #[cfg(unix)]
            return self.test_mode_via_stat(false, libc::S_IFLNK as _, vm);
            #[cfg(not(unix))]
            match &self.file_type {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => {
                    use crate::convert::ToPyException;
                    Err(e.to_pyexception(vm))
                }
                Ok(_) => Ok(false),
            }
        }

        #[pymethod]
        fn stat(
            &self,
            dir_fd: DirFd<'_, { STAT_DIR_FD as usize }>,
            follow_symlinks: FollowSymlinks,
            vm: &VirtualMachine,
        ) -> PyResult {
            // Use stored dir_fd if the caller didn't provide one
            let effective_dir_fd = if dir_fd == DirFd::default() {
                self.stat_dir_fd()
            } else {
                dir_fd
            };
            let do_stat = |follow_symlinks| {
                stat(
                    OsPath {
                        path: self.pathval.as_os_str().to_owned(),
                        origin: None,
                    }
                    .into(),
                    effective_dir_fd,
                    FollowSymlinks(follow_symlinks),
                    vm,
                )
            };
            let lstat = || match self.lstat.get() {
                Some(val) => Ok(val),
                None => {
                    let val = do_stat(false)?;
                    let _ = self.lstat.set(val);
                    Ok(self.lstat.get().unwrap())
                }
            };
            let stat = if follow_symlinks.0 {
                match self.stat.get() {
                    Some(val) => val,
                    None => {
                        let val = if self.is_symlink(vm)? {
                            do_stat(true)?
                        } else {
                            lstat()?.clone()
                        };
                        let _ = self.stat.set(val);
                        self.stat.get().unwrap()
                    }
                }
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
            Ok(host_nt::test_file_type_by_name(
                &self.pathval,
                host_nt::TestType::Junction,
            ))
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
            Err(vm.new_type_error("cannot pickle 'DirEntry' object"))
        }
    }

    impl Representable for DirEntry {
        #[inline]
        fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
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
                    let mut result = Wtf8Buf::from(format!("<{} ", zelf.class()));
                    result.push_wtf8(repr.as_wtf8());
                    result.push_char('>');
                    Ok(result)
                } else {
                    Err(vm.new_runtime_error(format!(
                        "reentrant call inside {}.__repr__",
                        zelf.class()
                    )))
                }
            } else {
                Ok(Wtf8Buf::from(format!("<{}>", zelf.class())))
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

    #[pyclass(flags(DISALLOW_INSTANTIATION), with(Destructor, IterNext, Iterable))]
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
            Err(vm.new_type_error("cannot pickle 'ScandirIterator' object"))
        }
    }
    impl Destructor for ScandirIterator {
        fn del(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            // Emit ResourceWarning if the iterator is not yet exhausted/closed
            if zelf.entries.read().is_some() {
                let _ = crate::stdlib::_warnings::warn(
                    vm.ctx.exceptions.resource_warning,
                    format!("unclosed scandir iterator {:?}", zelf.as_object()),
                    1,
                    vm,
                );
                zelf.close();
            }
            Ok(())
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
                                    host_nt::win32_xstat(pathval.as_os_str(), false)
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
                                    #[cfg(unix)]
                                    d_type: None,
                                    #[cfg(not(any(windows, target_os = "redox")))]
                                    dir_fd: None,
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

    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyattr]
    #[pyclass(name = "ScandirIter")]
    #[derive(Debug, PyPayload)]
    struct ScandirIteratorFd {
        dir: crate::common::lock::PyMutex<Option<host_posix::FdDirStream>>,
        /// The original fd passed to scandir(), stored in DirEntry for fstatat
        orig_fd: crt_fd::Raw,
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyclass(flags(DISALLOW_INSTANTIATION), with(Destructor, IterNext, Iterable))]
    impl ScandirIteratorFd {
        #[pymethod]
        fn close(&self) {
            let _dropped = self.dir.lock().take();
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
            Err(vm.new_type_error("cannot pickle 'ScandirIterator' object"))
        }
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    impl Destructor for ScandirIteratorFd {
        fn del(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            if zelf.dir.lock().is_some() {
                let _ = crate::stdlib::_warnings::warn(
                    vm.ctx.exceptions.resource_warning,
                    format!("unclosed scandir iterator {:?}", zelf.as_object()),
                    1,
                    vm,
                );
                zelf.close();
            }
            Ok(())
        }
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    impl SelfIter for ScandirIteratorFd {}

    #[cfg(all(unix, not(target_os = "redox")))]
    impl IterNext for ScandirIteratorFd {
        fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut guard = zelf.dir.lock();
            let dir = match guard.as_mut() {
                None => return Ok(PyIterReturn::StopIteration(None)),
                Some(dir) => dir,
            };
            let Some(entry) = dir.next_entry().map_err(|e| e.into_pyexception(vm))? else {
                drop(guard.take());
                return Ok(PyIterReturn::StopIteration(None));
            };
            let file_name = std::ffi::OsString::from(
                rustpython_host_env::os::bytes_as_os_str(&entry.name)
                    .expect("unix dir entry names are arbitrary bytes"),
            );
            let pathval = PathBuf::from(&file_name);
            Ok(PyIterReturn::Return(
                DirEntry {
                    file_name,
                    pathval,
                    file_type: Err(io::Error::other(
                        "file_type unavailable for fd-based scandir",
                    )),
                    d_type: entry.d_type,
                    dir_fd: Some(zelf.orig_fd),
                    mode: OutputMode::String,
                    lstat: OnceCell::new(),
                    stat: OnceCell::new(),
                    ino: AtomicCell::new(entry.ino as _),
                }
                .into_ref(&vm.ctx)
                .into(),
            ))
        }
    }

    #[pyfunction]
    fn scandir(path: OptionalArg<Option<OsPathOrFd<'_>>>, vm: &VirtualMachine) -> PyResult {
        let path = path
            .flatten()
            .unwrap_or_else(|| OsPathOrFd::Path(OsPath::new_str(".")));
        match path {
            OsPathOrFd::Path(path) => {
                let entries = crate::host_env::fs::read_dir(&path.path)
                    .map_err(|err| OSErrorBuilder::with_filename(&err, path.clone(), vm))?;
                Ok(ScandirIterator {
                    entries: PyRwLock::new(Some(entries)),
                    mode: path.mode(),
                }
                .into_ref(&vm.ctx)
                .into())
            }
            OsPathOrFd::Fd(fno) => {
                #[cfg(not(all(unix, not(target_os = "redox"))))]
                {
                    let _ = fno;
                    Err(vm.new_not_implemented_error("can't pass fd to scandir on this platform"))
                }
                #[cfg(all(unix, not(target_os = "redox")))]
                {
                    let dir = host_posix::FdDirStream::from_fd(fno.into())
                        .map_err(|e| e.into_pyexception(vm))?;
                    Ok(ScandirIteratorFd {
                        dir: crate::common::lock::PyMutex::new(Some(dir)),
                        orig_fd: fno.as_raw(),
                    }
                    .into_ref(&vm.ctx)
                    .into())
                }
            }
        }
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
        #[cfg_attr(target_env = "musl", allow(deprecated))]
        #[pyarg(positional, default)]
        #[pystruct_sequence(unnamed)]
        pub st_atime_int: libc::time_t,
        #[cfg_attr(target_env = "musl", allow(deprecated))]
        #[pyarg(positional, default)]
        #[pystruct_sequence(unnamed)]
        pub st_mtime_int: libc::time_t,
        #[cfg_attr(target_env = "musl", allow(deprecated))]
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
            #[allow(clippy::useless_conversion, reason = "needed for 32-bit platforms")]
            let st_blksize = i64::from(stat.st_blksize);
            #[cfg(not(windows))]
            #[allow(clippy::useless_conversion, reason = "needed for 32-bit platforms")]
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
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let seq: PyObjectRef = args.bind(vm)?;
            let result = crate::types::struct_sequence_new(cls.clone(), seq, vm)?;
            let tuple = result.downcast_ref::<PyTuple>().unwrap();
            let mut items: Vec<PyObjectRef> = tuple.to_vec();

            // Copy integer time fields to hidden float timestamp slots when not provided.
            // indices 7-9: st_atime_int, st_mtime_int, st_ctime_int
            // i+3: st_atime/st_mtime/st_ctime (float timestamps, copied from int if missing)
            // i+6: st_atime_ns/st_mtime_ns/st_ctime_ns (left as None if not provided)
            for i in 7..=9 {
                if vm.is_none(&items[i + 3]) {
                    items[i + 3] = items[i].clone();
                }
            }

            PyTuple::new_unchecked(items.into_boxed_slice())
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    #[cfg(windows)]
    fn stat_inner(
        file: OsPathOrFd<'_>,
        dir_fd: DirFd<'_, { STAT_DIR_FD as usize }>,
        follow_symlinks: FollowSymlinks,
    ) -> io::Result<Option<StatStruct>> {
        let [] = dir_fd.0;
        match file {
            OsPathOrFd::Path(path) => host_nt::win32_xstat(&path.path, follow_symlinks.0),
            OsPathOrFd::Fd(fd) => crate::host_env::fileutils::fstat(fd),
        }
        .map(Some)
    }

    #[cfg(not(windows))]
    fn stat_inner(
        file: OsPathOrFd<'_>,
        dir_fd: DirFd<'_, { STAT_DIR_FD as usize }>,
        follow_symlinks: FollowSymlinks,
    ) -> io::Result<Option<StatStruct>> {
        match file {
            OsPathOrFd::Path(path) => host_posix::stat_path(
                path.as_ref().as_os_str(),
                dir_fd.raw_opt(),
                follow_symlinks.0,
            ),
            OsPathOrFd::Fd(fd) => host_posix::stat_fd(fd).map(Some),
        }
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
        crate::host_env::os::current_dir().map_err(|err| err.into_pyexception(vm))
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
        crate::host_env::os::set_current_dir(&path.path)
            .map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))
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

        crate::host_env::os::rename(&src.path, &dst.path).map_err(|err| {
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
            crate::host_env::os::process_id()
        };
        vm.ctx.new_int(pid).into()
    }

    #[pyfunction]
    fn cpu_count(vm: &VirtualMachine) -> PyObjectRef {
        let cpu_count = crate::host_env::os::cpu_count();
        vm.ctx.new_int(cpu_count).into()
    }

    #[pyfunction]
    fn _exit(code: i32) {
        crate::host_env::os::exit(code)
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
        crate::host_env::os::isatty(fd)
    }

    #[pyfunction]
    pub fn lseek(
        fd: crt_fd::Borrowed<'_>,
        position: crt_fd::Offset,
        how: i32,
        vm: &VirtualMachine,
    ) -> PyResult<crt_fd::Offset> {
        crate::host_env::os::seek_fd(fd, position, how).map_err(|e| e.into_pyexception(vm))
    }

    #[derive(FromArgs)]
    struct LinkArgs {
        #[pyarg(any)]
        src: OsPath,
        #[pyarg(any)]
        dst: OsPath,
        #[pyarg(named, name = "follow_symlinks", optional)]
        follow_symlinks: OptionalArg<bool>,
    }

    #[pyfunction]
    fn link(args: LinkArgs, vm: &VirtualMachine) -> PyResult<()> {
        let LinkArgs {
            src,
            dst,
            follow_symlinks,
        } = args;

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let src_cstr = alloc::ffi::CString::new(src.path.as_os_str().as_bytes())
                .map_err(|_| vm.new_value_error("embedded null byte"))?;
            let dst_cstr = alloc::ffi::CString::new(dst.path.as_os_str().as_bytes())
                .map_err(|_| vm.new_value_error("embedded null byte"))?;

            let follow = follow_symlinks.into_option().unwrap_or(true);
            if let Err(err) =
                crate::host_env::posix::link_paths(src_cstr.as_c_str(), dst_cstr.as_c_str(), follow)
            {
                let builder = err.to_os_error_builder(vm);
                let builder = builder.filename(src.filename(vm));
                let builder = builder.filename2(dst.filename(vm));
                return Err(builder.build(vm).upcast());
            }

            Ok(())
        }

        #[cfg(not(unix))]
        {
            let src_path = match follow_symlinks.into_option() {
                Some(true) => {
                    // Explicit follow_symlinks=True: resolve symlinks
                    crate::host_env::fs::canonicalize(&src.path)
                        .unwrap_or_else(|_| PathBuf::from(src.path.clone()))
                }
                Some(false) | None => {
                    // Default or explicit no-follow: native hard_link behavior
                    PathBuf::from(src.path.clone())
                }
            };

            fs::hard_link(&src_path, &dst.path).map_err(|err| {
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
        let x = crate::host_env::os::system(cstr.as_c_str());
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
                if let Err(err) = crate::host_env::posix::set_file_times_at(
                    dir_fd.get().as_raw(),
                    path.as_c_str(),
                    acc,
                    modif,
                    _follow_symlinks.0,
                ) {
                    Err(OSErrorBuilder::with_filename(&err, path_for_err, vm))
                } else {
                    Ok(())
                }
            }
            #[cfg(target_os = "redox")]
            {
                let [] = dir_fd.0;
                rustpython_host_env::posix::utimes(path.as_ref(), acc, modif)
                    .map_err(|err| err.into_pyexception(vm))
            }
        }
        #[cfg(windows)]
        {
            let [] = dir_fd.0;

            if !_follow_symlinks.0 {
                return Err(vm.new_not_implemented_error(
                    "utime: follow_symlinks unavailable on this platform",
                ));
            }

            crate::host_env::os::set_file_times(&path, acc, modif)
                .map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))
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
            let times = crate::host_env::time::get_process_times_100ns()
                .ok_or_else(|| vm.new_last_os_error())?;

            let times_result = TimesResultData {
                user: times.user as f64 * 1e-7,
                system: times.system as f64 * 1e-7,
                children_user: 0.0,
                children_system: 0.0,
                elapsed: 0.0,
            };

            Ok(times_result.to_pyobject(vm))
        }
        #[cfg(unix)]
        {
            let times = crate::host_env::time::process_times()
                .map_err(|_| vm.new_os_error("Fail to get times".to_string()))?;

            let times_result = TimesResultData {
                user: times.user,
                system: times.system,
                children_user: times.children_user,
                children_system: times.children_system,
                elapsed: times.elapsed,
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
    fn copy_file_range(mut args: CopyFileRangeArgs<'_>, vm: &VirtualMachine) -> PyResult<usize> {
        let count: usize = args
            .count
            .try_into()
            .map_err(|_| vm.new_value_error("count should >= 0"))?;

        crate::host_env::os::copy_file_range(
            args.src,
            args.offset_src.as_mut(),
            args.dst,
            args.offset_dst.as_mut(),
            count,
        )
        .map_err(|_| vm.new_last_errno_error())
    }

    #[pyfunction]
    fn strerror(e: i32) -> String {
        crate::host_env::time::strerror(e)
    }

    #[pyfunction]
    pub fn ftruncate(fd: crt_fd::Borrowed<'_>, length: crt_fd::Offset) -> io::Result<()> {
        crt_fd::ftruncate(fd, length)
    }

    #[pyfunction]
    fn truncate(path: PyObjectRef, length: crt_fd::Offset, vm: &VirtualMachine) -> PyResult<()> {
        match path.clone().try_into_value::<crt_fd::Borrowed<'_>>(vm) {
            Ok(fd) => return ftruncate(fd, length).map_err(|e| e.into_pyexception(vm)),
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.warning) => return Err(e),
            Err(_) => {}
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
        let f = match crate::host_env::fs::open_write(&path) {
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
        let loadavg = crate::host_env::time::getloadavg()
            .map_err(|_| vm.new_os_error("Load averages are unobtainable".to_string()))?;
        Ok((loadavg[0], loadavg[1], loadavg[2]))
    }

    #[cfg(unix)]
    #[pyfunction]
    fn waitstatus_to_exitcode(status: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let status = u32::try_from(status)
            .map_err(|_| vm.new_value_error(format!("invalid WEXITSTATUS: {status}")))?;

        if let Some(exitcode) = crate::host_env::time::waitstatus_to_exitcode(status as libc::c_int)
        {
            return Ok(exitcode);
        }

        Err(vm.new_value_error(format!("Invalid wait status: {}", status as libc::c_int)))
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

        Ok(rustpython_host_env::os::device_encoding(fd))
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
        fn from_statvfs(st: crate::host_env::posix::StatVfsInfo) -> Self {
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
                f_fsid: st.f_fsid,
            }
        }
    }

    /// Perform a statvfs system call on the given path.
    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyfunction]
    #[pyfunction(name = "fstatvfs")]
    fn statvfs(path: OsPathOrFd<'_>, vm: &VirtualMachine) -> PyResult {
        let st = match &path {
            OsPathOrFd::Path(p) => {
                let cpath = p.clone().into_cstring(vm)?;
                crate::host_env::posix::statvfs_path(cpath.as_c_str())
            }
            OsPathOrFd::Fd(fd) => crate::host_env::posix::statvfs_fd(fd.as_raw()),
        };
        if let Err(err) = st {
            return Err(OSErrorBuilder::with_filename(&err, path, vm));
        }
        Ok(StatvfsResultData::from_statvfs(st.unwrap()).to_pyobject(vm))
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
            SupportFunc::new("remove", Some(false), Some(UNLINK_DIR_FD), Some(false)),
            SupportFunc::new("unlink", Some(false), Some(UNLINK_DIR_FD), Some(false)),
            SupportFunc::new("rename", Some(false), None, Some(false)),
            SupportFunc::new("replace", Some(false), None, Some(false)), // TODO: Fix replace
            SupportFunc::new("rmdir", Some(false), Some(RMDIR_DIR_FD), Some(false)),
            SupportFunc::new("scandir", Some(SCANDIR_FD), Some(false), Some(false)),
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
pub(crate) use _os::ftruncate;

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

/// Convert a mapping (e.g. os._Environ) to a plain dict for use by execve/posix_spawn.
///
/// For `os._Environ`, accesses the internal `_data` dict directly at the Rust level.
/// This avoids Python-level method calls that can deadlock after fork() when
/// parking_lot locks are held by threads that no longer exist.
#[cfg(any(unix, windows))]
pub(crate) fn envobj_to_dict(
    env: crate::function::ArgMapping,
    vm: &VirtualMachine,
) -> PyResult<crate::builtins::PyDictRef> {
    let obj = env.obj();
    if let Some(dict) = obj.downcast_ref_if_exact::<crate::builtins::PyDict>(vm) {
        return Ok(dict.to_owned());
    }
    if let Some(inst_dict) = obj.dict()
        && let Ok(Some(data)) = inst_dict.get_item_opt("_data", vm)
        && let Some(dict) = data.downcast_ref_if_exact::<crate::builtins::PyDict>(vm)
    {
        return Ok(dict.to_owned());
    }
    let keys = vm.call_method(obj, "keys", ())?;
    let dict = vm.ctx.new_dict();
    for key in keys.get_iter(vm)?.into_iter::<PyObjectRef>(vm)? {
        let key = key?;
        let val = obj.get_item(&*key, vm)?;
        dict.set_item(&*key, val, vm)?;
    }
    Ok(dict)
}

#[cfg(not(windows))]
use super::posix as platform;

#[cfg(windows)]
use super::nt as platform;

pub(crate) use platform::module::MODULE_NAME;
