use std::ffi;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
use std::sync::RwLock;
use std::time::{Duration, SystemTime};
use std::{env, fs};

use bitflags::bitflags;
use crossbeam_utils::atomic::AtomicCell;
#[cfg(unix)]
use nix::errno::Errno;
#[cfg(all(unix, not(target_os = "redox")))]
use nix::pty::openpty;
#[cfg(unix)]
use nix::unistd::{self, Gid, Pid, Uid};
#[cfg(unix)]
use std::os::unix::io::RawFd;
#[cfg(windows)]
use std::os::windows::io::RawHandle;

use super::errno::errors;
use crate::exceptions::PyBaseExceptionRef;
use crate::function::{IntoPyNativeFunc, OptionalArg, PyFuncArgs};
use crate::obj::objbyteinner::PyBytesLike;
use crate::obj::objbytes::{PyBytes, PyBytesRef};
use crate::obj::objdict::{PyDictRef, PyMapping};
use crate::obj::objint::PyIntRef;
use crate::obj::objiter;
use crate::obj::objset::PySet;
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::objtuple::PyTupleRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    Either, ItemProtocol, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    TypeProtocol,
};
use crate::vm::VirtualMachine;

// just to avoid unused import warnings
#[cfg(unix)]
use crate::pyobject::PyIterable;
#[cfg(unix)]
use std::convert::TryFrom;

#[cfg(windows)]
pub const MODULE_NAME: &str = "nt";
#[cfg(not(windows))]
pub const MODULE_NAME: &str = "posix";

#[cfg(unix)]
pub fn raw_file_number(handle: File) -> i64 {
    use std::os::unix::io::IntoRawFd;

    i64::from(handle.into_raw_fd())
}

#[cfg(unix)]
pub fn rust_file(raw_fileno: i64) -> File {
    use std::os::unix::io::FromRawFd;

    unsafe { File::from_raw_fd(raw_fileno as i32) }
}

#[cfg(windows)]
pub fn raw_file_number(handle: File) -> i64 {
    use std::os::windows::io::IntoRawHandle;

    handle.into_raw_handle() as i64
}

#[cfg(windows)]
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

#[cfg(target_os = "wasi")]
pub fn raw_file_number(handle: File) -> i64 {
    // This should be safe, since the wasi api is pretty well defined, but once
    // `wasi_ext` get's stabilized, we should use that instead.
    unsafe { std::mem::transmute::<_, u32>(handle).into() }
}

#[cfg(target_os = "wasi")]
pub fn rust_file(raw_fileno: i64) -> File {
    unsafe { std::mem::transmute(raw_fileno as u32) }
}

#[cfg(not(any(unix, windows, target_os = "wasi")))]
pub fn rust_file(raw_fileno: i64) -> File {
    unimplemented!();
}

#[cfg(not(any(unix, windows, target_os = "wasi")))]
pub fn raw_file_number(handle: File) -> i64 {
    unimplemented!();
}

#[derive(Debug, Copy, Clone)]
enum OutputMode {
    String,
    Bytes,
}

fn output_by_mode(val: String, mode: OutputMode, vm: &VirtualMachine) -> PyObjectRef {
    match mode {
        OutputMode::String => vm.ctx.new_str(val),
        OutputMode::Bytes => vm.ctx.new_bytes(val.as_bytes().to_vec()),
    }
}

pub struct PyPathLike {
    path: ffi::OsString,
    mode: OutputMode,
}

impl PyPathLike {
    pub fn new_str(path: String) -> Self {
        PyPathLike {
            path: ffi::OsString::from(path),
            mode: OutputMode::String,
        }
    }
    #[cfg(windows)]
    pub fn wide(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.path.encode_wide().chain(std::iter::once(0)).collect()
    }
}

impl TryFromObject for PyPathLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match_class!(match obj.clone() {
            l @ PyString => {
                Ok(PyPathLike {
                    path: ffi::OsString::from(l.as_str()),
                    mode: OutputMode::String,
                })
            }
            i @ PyBytes => {
                Ok(PyPathLike {
                    path: bytes_as_osstr(&i, vm)?.to_os_string(),
                    mode: OutputMode::Bytes,
                })
            }
            obj => {
                let method = vm.get_method_or_type_error(obj.clone(), "__fspath__", || {
                    format!(
                        "expected str, bytes or os.PathLike object, not '{}'",
                        obj.class().name
                    )
                })?;
                let result = vm.invoke(&method, PyFuncArgs::default())?;
                match_class!(match result.clone() {
                    l @ PyString => {
                        Ok(PyPathLike {
                            path: ffi::OsString::from(l.as_str()),
                            mode: OutputMode::String,
                        })
                    }
                    i @ PyBytes => {
                        Ok(PyPathLike {
                            path: bytes_as_osstr(&i, vm)?.to_os_string(),
                            mode: OutputMode::Bytes,
                        })
                    }
                    _ => Err(vm.new_type_error(format!(
                        "expected {}.__fspath__() to return str or bytes, not '{}'",
                        obj.class().name,
                        result.class().name
                    ))),
                })
            }
        })
    }
}

fn make_path<'a>(_vm: &VirtualMachine, path: &'a PyPathLike, dir_fd: &DirFd) -> &'a ffi::OsStr {
    if dir_fd.dir_fd.is_some() {
        unimplemented!();
    } else {
        path.path.as_os_str()
    }
}

fn os_close(fileno: i64) {
    //The File type automatically closes when it goes out of scope.
    //To enable us to close these file descriptors (and hence prevent leaks)
    //we seek to create the relevant File and simply let it pass out of scope!
    rust_file(fileno);
}

#[cfg(unix)]
type OpenFlags = i32;
#[cfg(windows)]
type OpenFlags = u32;
#[cfg(target_os = "wasi")]
type OpenFlags = u16;

#[cfg(any(unix, windows, target_os = "wasi"))]
pub fn os_open(
    name: PyPathLike,
    flags: OpenFlags,
    _mode: OptionalArg<PyIntRef>,
    dir_fd: OptionalArg<PyIntRef>,
    vm: &VirtualMachine,
) -> PyResult<i64> {
    let dir_fd = DirFd {
        dir_fd: dir_fd.into_option(),
    };
    let fname = make_path(vm, &name, &dir_fd);

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
    options.custom_flags(flags);
    let handle = options
        .open(fname)
        .map_err(|err| convert_io_error(vm, err))?;

    Ok(raw_file_number(handle))
}

#[cfg(not(any(unix, windows, target_os = "wasi")))]
pub fn os_open(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    unimplemented!()
}

pub fn convert_io_error(vm: &VirtualMachine, err: io::Error) -> PyBaseExceptionRef {
    #[allow(unreachable_patterns)] // some errors are just aliases of each other
    let exc_type = match err.kind() {
        ErrorKind::NotFound => vm.ctx.exceptions.file_not_found_error.clone(),
        ErrorKind::PermissionDenied => vm.ctx.exceptions.permission_error.clone(),
        ErrorKind::AlreadyExists => vm.ctx.exceptions.file_exists_error.clone(),
        ErrorKind::WouldBlock => vm.ctx.exceptions.blocking_io_error.clone(),
        _ => match err.raw_os_error() {
            Some(errors::EAGAIN)
            | Some(errors::EALREADY)
            | Some(errors::EWOULDBLOCK)
            | Some(errors::EINPROGRESS) => vm.ctx.exceptions.blocking_io_error.clone(),
            _ => vm.ctx.exceptions.os_error.clone(),
        },
    };
    let os_error = vm.new_exception_msg(exc_type, err.to_string());
    let errno = match err.raw_os_error() {
        Some(errno) => vm.new_int(errno),
        None => vm.get_none(),
    };
    vm.set_attr(os_error.as_object(), "errno", errno).unwrap();
    os_error
}

#[cfg(unix)]
pub fn convert_nix_error(vm: &VirtualMachine, err: nix::Error) -> PyBaseExceptionRef {
    let nix_error = match err {
        nix::Error::InvalidPath => {
            let exc_type = vm.ctx.exceptions.file_not_found_error.clone();
            vm.new_exception_msg(exc_type, err.to_string())
        }
        nix::Error::InvalidUtf8 => {
            let exc_type = vm.ctx.exceptions.unicode_error.clone();
            vm.new_exception_msg(exc_type, err.to_string())
        }
        nix::Error::UnsupportedOperation => vm.new_runtime_error(err.to_string()),
        nix::Error::Sys(errno) => {
            let exc_type = convert_nix_errno(vm, errno);
            vm.new_exception_msg(exc_type, err.to_string())
        }
    };

    if let nix::Error::Sys(errno) = err {
        vm.set_attr(nix_error.as_object(), "errno", vm.ctx.new_int(errno as i32))
            .unwrap();
    }

    nix_error
}

#[cfg(unix)]
fn convert_nix_errno(vm: &VirtualMachine, errno: Errno) -> PyClassRef {
    match errno {
        Errno::EPERM => vm.ctx.exceptions.permission_error.clone(),
        _ => vm.ctx.exceptions.os_error.clone(),
    }
}

/// Convert the error stored in the `errno` variable into an Exception
#[inline]
pub fn errno_err(vm: &VirtualMachine) -> PyBaseExceptionRef {
    convert_io_error(vm, io::Error::last_os_error())
}

// Flags for os_access
bitflags! {
    pub struct AccessFlags: u8{
        const F_OK = 0;
        const R_OK = 4;
        const W_OK = 2;
        const X_OK = 1;
    }
}

#[cfg(unix)]
struct Permissions {
    is_readable: bool,
    is_writable: bool,
    is_executable: bool,
}

#[cfg(unix)]
fn get_permissions(mode: u32) -> Permissions {
    Permissions {
        is_readable: mode & 4 != 0,
        is_writable: mode & 2 != 0,
        is_executable: mode & 1 != 0,
    }
}

#[cfg(unix)]
fn get_right_permission(
    mode: u32,
    file_owner: Uid,
    file_group: Gid,
) -> Result<Permissions, nix::Error> {
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

#[cfg(target_os = "macos")]
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

#[cfg(any(target_os = "linux", target_os = "android", target_os = "openbsd"))]
use nix::unistd::getgroups;

#[cfg(target_os = "redox")]
fn getgroups() -> nix::Result<Vec<Gid>> {
    unimplemented!("redox getgroups")
}

#[cfg(unix)]
fn os_access(path: PyPathLike, mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
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

    let metadata = metadata.map_err(|err| convert_io_error(vm, err))?;

    let user_id = metadata.uid();
    let group_id = metadata.gid();
    let mode = metadata.mode();

    let perm = get_right_permission(mode, Uid::from_raw(user_id), Gid::from_raw(group_id))
        .map_err(|err| convert_nix_error(vm, err))?;

    let r_ok = !flags.contains(AccessFlags::R_OK) || perm.is_readable;
    let w_ok = !flags.contains(AccessFlags::W_OK) || perm.is_writable;
    let x_ok = !flags.contains(AccessFlags::X_OK) || perm.is_executable;

    Ok(r_ok && w_ok && x_ok)
}
#[cfg(windows)]
fn os_access(path: PyPathLike, mode: u8) -> bool {
    use winapi::um::{fileapi, winnt};
    let attr = unsafe { fileapi::GetFileAttributesW(path.wide().as_ptr()) };
    attr != fileapi::INVALID_FILE_ATTRIBUTES
        && (mode & 2 == 0
            || attr & winnt::FILE_ATTRIBUTE_READONLY == 0
            || attr & winnt::FILE_ATTRIBUTE_DIRECTORY != 0)
}
#[cfg(not(any(unix, windows)))]
fn os_access(path: PyStringRef, mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
    unimplemented!()
}

fn os_error(message: OptionalArg<PyStringRef>, vm: &VirtualMachine) -> PyResult {
    let msg = message.map_or("".to_owned(), |msg| msg.as_str().to_owned());

    Err(vm.new_os_error(msg))
}

fn os_fsync(fd: i64, vm: &VirtualMachine) -> PyResult<()> {
    let file = rust_file(fd);
    file.sync_all().map_err(|err| convert_io_error(vm, err))?;
    // Avoid closing the fd
    raw_file_number(file);
    Ok(())
}

fn os_read(fd: i64, n: usize, vm: &VirtualMachine) -> PyResult {
    let mut buffer = vec![0u8; n];
    let mut file = rust_file(fd);
    let n = file
        .read(&mut buffer)
        .map_err(|err| convert_io_error(vm, err))?;
    buffer.truncate(n);

    // Avoid closing the fd
    raw_file_number(file);
    Ok(vm.ctx.new_bytes(buffer))
}

fn os_write(fd: i64, data: PyBytesLike, vm: &VirtualMachine) -> PyResult {
    let mut file = rust_file(fd);
    let written = data
        .with_ref(|b| file.write(b))
        .map_err(|err| convert_io_error(vm, err))?;

    // Avoid closing the fd
    raw_file_number(file);
    Ok(vm.ctx.new_int(written))
}

fn os_remove(path: PyPathLike, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult<()> {
    let path = make_path(vm, &path, &dir_fd);
    fs::remove_file(path).map_err(|err| convert_io_error(vm, err))
}

fn os_mkdir(
    path: PyPathLike,
    _mode: OptionalArg<PyIntRef>,
    dir_fd: DirFd,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let path = make_path(vm, &path, &dir_fd);
    fs::create_dir(path).map_err(|err| convert_io_error(vm, err))
}

fn os_mkdirs(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    fs::create_dir_all(path.as_str()).map_err(|err| convert_io_error(vm, err))
}

fn os_rmdir(path: PyPathLike, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult<()> {
    let path = make_path(vm, &path, &dir_fd);
    fs::remove_dir(path).map_err(|err| convert_io_error(vm, err))
}

fn os_listdir(path: PyPathLike, vm: &VirtualMachine) -> PyResult {
    let res: PyResult<Vec<PyObjectRef>> = fs::read_dir(&path.path)
        .map_err(|err| convert_io_error(vm, err))?
        .map(|entry| match entry {
            Ok(entry_path) => Ok(output_by_mode(
                entry_path.file_name().into_string().unwrap(),
                path.mode,
                vm,
            )),
            Err(s) => Err(convert_io_error(vm, s)),
        })
        .collect();
    Ok(vm.ctx.new_list(res?))
}

fn bytes_as_osstr<'a>(b: &'a [u8], vm: &VirtualMachine) -> PyResult<&'a ffi::OsStr> {
    let os_str = {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Some(ffi::OsStr::from_bytes(b))
        }
        #[cfg(not(unix))]
        {
            std::str::from_utf8(b).ok().map(|s| s.as_ref())
        }
    };
    os_str
        .ok_or_else(|| vm.new_value_error("Can't convert bytes to str for env function".to_owned()))
}

fn os_putenv(
    key: Either<PyStringRef, PyBytesRef>,
    value: Either<PyStringRef, PyBytesRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let key: &ffi::OsStr = match key {
        Either::A(ref s) => s.as_str().as_ref(),
        Either::B(ref b) => bytes_as_osstr(b.get_value(), vm)?,
    };
    let value: &ffi::OsStr = match value {
        Either::A(ref s) => s.as_str().as_ref(),
        Either::B(ref b) => bytes_as_osstr(b.get_value(), vm)?,
    };
    env::set_var(key, value);
    Ok(())
}

fn os_unsetenv(key: Either<PyStringRef, PyBytesRef>, vm: &VirtualMachine) -> PyResult<()> {
    let key: &ffi::OsStr = match key {
        Either::A(ref s) => s.as_str().as_ref(),
        Either::B(ref b) => bytes_as_osstr(b.get_value(), vm)?,
    };
    env::remove_var(key);
    Ok(())
}

fn _os_environ(vm: &VirtualMachine) -> PyDictRef {
    let environ = vm.ctx.new_dict();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        for (key, value) in env::vars_os() {
            environ
                .set_item(
                    &vm.ctx.new_bytes(key.into_vec()),
                    vm.ctx.new_bytes(value.into_vec()),
                    vm,
                )
                .unwrap();
        }
    }
    #[cfg(windows)]
    {
        for (key, value) in env::vars() {
            environ
                .set_item(&vm.new_str(key), vm.new_str(value), vm)
                .unwrap();
        }
    }
    environ
}

fn os_readlink(path: PyPathLike, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult {
    let path = make_path(vm, &path, &dir_fd);
    let path = fs::read_link(path).map_err(|err| convert_io_error(vm, err))?;
    let path = path.into_os_string().into_string().map_err(|_osstr| {
        vm.new_unicode_decode_error("Can't convert OS path to valid UTF-8 string".into())
    })?;
    Ok(vm.ctx.new_str(path))
}

#[derive(Debug)]
struct DirEntry {
    entry: fs::DirEntry,
    mode: OutputMode,
}

type DirEntryRef = PyRef<DirEntry>;

impl PyValue for DirEntry {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class(MODULE_NAME, "DirEntry")
    }
}

#[derive(FromArgs, Default)]
struct DirFd {
    #[pyarg(keyword_only, default = "None")]
    dir_fd: Option<PyIntRef>,
}

#[derive(FromArgs)]
struct FollowSymlinks {
    #[pyarg(keyword_only, default = "true")]
    follow_symlinks: bool,
}

impl DirEntryRef {
    fn name(self, vm: &VirtualMachine) -> PyObjectRef {
        let file_name = self.entry.file_name().into_string().unwrap();
        output_by_mode(file_name, self.mode, vm)
    }

    fn path(self, vm: &VirtualMachine) -> PyObjectRef {
        let path = self.entry.path().to_str().unwrap().to_owned();
        output_by_mode(path, self.mode, vm)
    }

    #[allow(clippy::match_bool)]
    fn perform_on_metadata(
        self,
        follow_symlinks: FollowSymlinks,
        action: fn(fs::Metadata) -> bool,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let metadata = match follow_symlinks.follow_symlinks {
            true => fs::metadata(self.entry.path()),
            false => fs::symlink_metadata(self.entry.path()),
        };
        let meta = metadata.map_err(|err| convert_io_error(vm, err))?;
        Ok(action(meta))
    }

    fn is_dir(self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<bool> {
        self.perform_on_metadata(
            follow_symlinks,
            |meta: fs::Metadata| -> bool { meta.is_dir() },
            vm,
        )
    }

    fn is_file(self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<bool> {
        self.perform_on_metadata(
            follow_symlinks,
            |meta: fs::Metadata| -> bool { meta.is_file() },
            vm,
        )
    }

    fn is_symlink(self, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(self
            .entry
            .file_type()
            .map_err(|err| convert_io_error(vm, err))?
            .is_symlink())
    }

    fn stat(self, dir_fd: DirFd, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult {
        os_stat(
            Either::A(PyPathLike {
                path: self.entry.path().into_os_string(),
                mode: OutputMode::String,
            }),
            dir_fd,
            follow_symlinks,
            vm,
        )
    }
}

#[pyclass]
#[derive(Debug)]
struct ScandirIterator {
    entries: RwLock<fs::ReadDir>,
    exhausted: AtomicCell<bool>,
    mode: OutputMode,
}

impl PyValue for ScandirIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class(MODULE_NAME, "ScandirIter")
    }
}

#[pyimpl]
impl ScandirIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if self.exhausted.load() {
            return Err(objiter::new_stop_iteration(vm));
        }

        match self.entries.write().unwrap().next() {
            Some(entry) => match entry {
                Ok(entry) => Ok(DirEntry {
                    entry,
                    mode: self.mode,
                }
                .into_ref(vm)
                .into_object()),
                Err(s) => Err(convert_io_error(vm, s)),
            },
            None => {
                self.exhausted.store(true);
                Err(objiter::new_stop_iteration(vm))
            }
        }
    }

    #[pymethod]
    fn close(&self) {
        self.exhausted.store(true);
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__enter__")]
    fn enter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__exit__")]
    fn exit(zelf: PyRef<Self>, _args: PyFuncArgs) {
        zelf.close()
    }
}

fn os_scandir(path: OptionalArg<PyPathLike>, vm: &VirtualMachine) -> PyResult {
    let path = match path {
        OptionalArg::Present(path) => path,
        OptionalArg::Missing => PyPathLike::new_str(".".to_owned()),
    };

    match fs::read_dir(path.path) {
        Ok(iter) => Ok(ScandirIterator {
            entries: RwLock::new(iter),
            exhausted: AtomicCell::new(false),
            mode: path.mode,
        }
        .into_ref(vm)
        .into_object()),
        Err(s) => Err(convert_io_error(vm, s)),
    }
}

#[pystruct_sequence(name = "os.stat_result")]
#[derive(Debug)]
struct StatResult {
    st_mode: u32,
    st_ino: u64,
    st_dev: u64,
    st_nlink: u64,
    st_uid: u32,
    st_gid: u32,
    st_size: u64,
    st_atime: f64,
    st_mtime: f64,
    st_ctime: f64,
}

impl StatResult {
    fn into_obj(self, vm: &VirtualMachine) -> PyObjectRef {
        self.into_struct_sequence(vm, vm.class(MODULE_NAME, "stat_result"))
            .unwrap()
            .into_object()
    }
}

// Copied code from Duration::as_secs_f64 as it's still unstable
fn duration_as_secs_f64(duration: Duration) -> f64 {
    (duration.as_secs() as f64) + f64::from(duration.subsec_nanos()) / 1_000_000_000_f64
}

fn to_seconds_from_unix_epoch(sys_time: SystemTime) -> f64 {
    match sys_time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => duration_as_secs_f64(duration),
        Err(err) => -duration_as_secs_f64(err.duration()),
    }
}

#[cfg(unix)]
fn to_seconds_from_nanos(secs: i64, nanos: i64) -> f64 {
    let duration = Duration::new(secs as u64, nanos as u32);
    duration_as_secs_f64(duration)
}

#[cfg(unix)]
fn os_stat(
    file: Either<PyPathLike, i64>,
    dir_fd: DirFd,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult {
    #[cfg(target_os = "android")]
    use std::os::android::fs::MetadataExt;
    #[cfg(target_os = "linux")]
    use std::os::linux::fs::MetadataExt;
    #[cfg(target_os = "macos")]
    use std::os::macos::fs::MetadataExt;
    #[cfg(target_os = "openbsd")]
    use std::os::openbsd::fs::MetadataExt;
    #[cfg(target_os = "redox")]
    use std::os::redox::fs::MetadataExt;

    let get_stats = move || -> io::Result<PyObjectRef> {
        let meta = match file {
            Either::A(path) => {
                let path = make_path(vm, &path, &dir_fd);
                if follow_symlinks.follow_symlinks {
                    fs::metadata(path)?
                } else {
                    fs::symlink_metadata(path)?
                }
            }
            Either::B(fno) => {
                let file = rust_file(fno);
                let meta = file.metadata()?;
                raw_file_number(file);
                meta
            }
        };

        Ok(StatResult {
            st_mode: meta.st_mode(),
            st_ino: meta.st_ino(),
            st_dev: meta.st_dev(),
            st_nlink: meta.st_nlink(),
            st_uid: meta.st_uid(),
            st_gid: meta.st_gid(),
            st_size: meta.st_size(),
            st_atime: to_seconds_from_unix_epoch(meta.accessed()?),
            st_mtime: to_seconds_from_unix_epoch(meta.modified()?),
            st_ctime: to_seconds_from_nanos(meta.st_ctime(), meta.st_ctime_nsec()),
        }
        .into_obj(vm))
    };

    get_stats().map_err(|err| convert_io_error(vm, err))
}

// Copied from CPython fileutils.c
#[cfg(windows)]
fn attributes_to_mode(attr: u32) -> u32 {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 16;
    const FILE_ATTRIBUTE_READONLY: u32 = 1;
    const S_IFDIR: u32 = 0o040000;
    const S_IFREG: u32 = 0o100000;
    let mut m: u32 = 0;
    if attr & FILE_ATTRIBUTE_DIRECTORY == FILE_ATTRIBUTE_DIRECTORY {
        m |= S_IFDIR | 0111; /* IFEXEC for user,group,other */
    } else {
        m |= S_IFREG;
    }
    if attr & FILE_ATTRIBUTE_READONLY == FILE_ATTRIBUTE_READONLY {
        m |= 0444;
    } else {
        m |= 0666;
    }
    m
}

#[cfg(windows)]
fn os_stat(
    file: Either<PyPathLike, i64>,
    _dir_fd: DirFd, // TODO: error
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult {
    use std::os::windows::fs::MetadataExt;

    let get_stats = move || -> io::Result<PyObjectRef> {
        let meta = match file {
            Either::A(path) => match follow_symlinks.follow_symlinks {
                true => fs::metadata(path.path)?,
                false => fs::symlink_metadata(path.path)?,
            },
            Either::B(fno) => {
                let f = rust_file(fno);
                let meta = f.metadata()?;
                raw_file_number(f);
                meta
            }
        };

        Ok(StatResult {
            st_mode: attributes_to_mode(meta.file_attributes()),
            st_ino: 0,   // TODO: Not implemented in std::os::windows::fs::MetadataExt.
            st_dev: 0,   // TODO: Not implemented in std::os::windows::fs::MetadataExt.
            st_nlink: 0, // TODO: Not implemented in std::os::windows::fs::MetadataExt.
            st_uid: 0,   // 0 on windows
            st_gid: 0,   // 0 on windows
            st_size: meta.file_size(),
            st_atime: to_seconds_from_unix_epoch(meta.accessed()?),
            st_mtime: to_seconds_from_unix_epoch(meta.modified()?),
            st_ctime: to_seconds_from_unix_epoch(meta.created()?),
        }
        .into_obj(vm))
    };

    get_stats().map_err(|e| convert_io_error(vm, e))
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    target_os = "redox",
    windows,
    unix
)))]
fn os_stat(
    file: Either<PyPathLike, i64>,
    _dir_fd: DirFd,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult {
    unimplemented!();
}

fn os_lstat(file: Either<PyPathLike, i64>, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult {
    os_stat(
        file,
        dir_fd,
        FollowSymlinks {
            follow_symlinks: false,
        },
        vm,
    )
}

#[cfg(unix)]
fn os_symlink(
    src: PyPathLike,
    dst: PyPathLike,
    dir_fd: DirFd,
    vm: &VirtualMachine,
) -> PyResult<()> {
    use std::os::unix::fs as unix_fs;
    let dst = make_path(vm, &dst, &dir_fd);
    unix_fs::symlink(src.path, dst).map_err(|err| convert_io_error(vm, err))
}

#[cfg(windows)]
fn os_symlink(
    src: PyPathLike,
    dst: PyPathLike,
    _dir_fd: DirFd,
    vm: &VirtualMachine,
) -> PyResult<()> {
    use std::os::windows::fs as win_fs;
    let meta = fs::metadata(src.path.clone()).map_err(|err| convert_io_error(vm, err))?;
    let ret = if meta.is_file() {
        win_fs::symlink_file(src.path, dst.path)
    } else if meta.is_dir() {
        win_fs::symlink_dir(src.path, dst.path)
    } else {
        panic!("Uknown file type");
    };
    ret.map_err(|err| convert_io_error(vm, err))
}

#[cfg(all(not(unix), not(windows)))]
fn os_symlink(
    src: PyPathLike,
    dst: PyPathLike,
    dir_fd: DirFd,
    vm: &VirtualMachine,
) -> PyResult<()> {
    unimplemented!();
}

fn os_getcwd(vm: &VirtualMachine) -> PyResult<String> {
    Ok(env::current_dir()
        .map_err(|err| convert_io_error(vm, err))?
        .as_path()
        .to_str()
        .unwrap()
        .to_owned())
}

fn os_chdir(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    env::set_current_dir(path.as_str()).map_err(|err| convert_io_error(vm, err))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn os_chroot(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    nix::unistd::chroot(path.as_str()).map_err(|err| convert_nix_error(vm, err))
}

#[cfg(unix)]
fn os_get_inheritable(fd: RawFd, vm: &VirtualMachine) -> PyResult<bool> {
    use nix::fcntl::fcntl;
    use nix::fcntl::FcntlArg;
    let flags = fcntl(fd, FcntlArg::F_GETFD);
    match flags {
        Ok(ret) => Ok((ret & libc::FD_CLOEXEC) == 0),
        Err(err) => Err(convert_nix_error(vm, err)),
    }
}

fn os_set_inheritable(fd: i64, inheritable: bool, vm: &VirtualMachine) -> PyResult<()> {
    #[cfg(not(any(unix, windows)))]
    {
        unimplemented!()
    }
    #[cfg(unix)]
    {
        let fd = fd as RawFd;
        let _set_flag = || {
            use nix::fcntl::fcntl;
            use nix::fcntl::FcntlArg;
            use nix::fcntl::FdFlag;

            let flags = FdFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFD)?);
            let mut new_flags = flags;
            new_flags.set(FdFlag::from_bits_truncate(libc::FD_CLOEXEC), !inheritable);
            if flags != new_flags {
                fcntl(fd, FcntlArg::F_SETFD(new_flags))?;
            }
            Ok(())
        };
        _set_flag().or_else(|err| Err(convert_nix_error(vm, err)))
    }
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

#[cfg(unix)]
fn os_get_blocking(fd: RawFd, vm: &VirtualMachine) -> PyResult<bool> {
    use nix::fcntl::fcntl;
    use nix::fcntl::FcntlArg;
    let flags = fcntl(fd, FcntlArg::F_GETFL);
    match flags {
        Ok(ret) => Ok((ret & libc::O_NONBLOCK) == 0),
        Err(err) => Err(convert_nix_error(vm, err)),
    }
}

#[cfg(unix)]
fn os_set_blocking(fd: RawFd, blocking: bool, vm: &VirtualMachine) -> PyResult<()> {
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
    _set_flag().or_else(|err| Err(convert_nix_error(vm, err)))
}

#[cfg(unix)]
fn os_pipe(vm: &VirtualMachine) -> PyResult<(RawFd, RawFd)> {
    use nix::unistd::close;
    use nix::unistd::pipe;
    let (rfd, wfd) = pipe().map_err(|err| convert_nix_error(vm, err))?;
    os_set_inheritable(rfd.into(), false, vm)
        .and_then(|_| os_set_inheritable(wfd.into(), false, vm))
        .or_else(|err| {
            let _ = close(rfd);
            let _ = close(wfd);
            Err(err)
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
fn os_pipe2(flags: libc::c_int, vm: &VirtualMachine) -> PyResult<(RawFd, RawFd)> {
    use nix::fcntl::OFlag;
    use nix::unistd::pipe2;
    let oflags = OFlag::from_bits_truncate(flags);
    pipe2(oflags).map_err(|err| convert_nix_error(vm, err))
}

#[cfg(unix)]
fn os_system(command: PyStringRef) -> PyResult<i32> {
    use libc::system;
    use std::ffi::CString;

    let rstr = command.as_str();
    let cstr = CString::new(rstr).unwrap();
    let x = unsafe { system(cstr.as_ptr()) };
    Ok(x)
}

#[cfg(unix)]
fn os_chmod(
    path: PyPathLike,
    dir_fd: DirFd,
    mode: u32,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = make_path(vm, &path, &dir_fd);
    let metadata = if follow_symlinks.follow_symlinks {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    };
    let meta = metadata.map_err(|err| convert_io_error(vm, err))?;
    let mut permissions = meta.permissions();
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions).map_err(|err| convert_io_error(vm, err))?;
    Ok(())
}

fn os_fspath(path: PyPathLike, vm: &VirtualMachine) -> PyObjectRef {
    output_by_mode(path.path.to_str().unwrap().to_owned(), path.mode, vm)
}

fn os_rename(src: PyPathLike, dst: PyPathLike, vm: &VirtualMachine) -> PyResult<()> {
    fs::rename(src.path, dst.path).map_err(|err| convert_io_error(vm, err))
}

fn os_getpid(vm: &VirtualMachine) -> PyObjectRef {
    let pid = std::process::id();
    vm.new_int(pid)
}

fn os_cpu_count(vm: &VirtualMachine) -> PyObjectRef {
    let cpu_count = num_cpus::get();
    vm.new_int(cpu_count)
}

fn os_exit(code: i32) {
    std::process::exit(code)
}

#[cfg(unix)]
fn os_getppid(vm: &VirtualMachine) -> PyObjectRef {
    let ppid = unistd::getppid().as_raw();
    vm.new_int(ppid)
}

#[cfg(unix)]
fn os_getgid(vm: &VirtualMachine) -> PyObjectRef {
    let gid = unistd::getgid().as_raw();
    vm.new_int(gid)
}

#[cfg(unix)]
fn os_getegid(vm: &VirtualMachine) -> PyObjectRef {
    let egid = unistd::getegid().as_raw();
    vm.new_int(egid)
}

#[cfg(unix)]
fn os_getpgid(pid: u32, vm: &VirtualMachine) -> PyResult {
    match unistd::getpgid(Some(Pid::from_raw(pid as i32))) {
        Ok(pgid) => Ok(vm.new_int(pgid.as_raw())),
        Err(err) => Err(convert_nix_error(vm, err)),
    }
}

#[cfg(unix)]
fn os_getpgrp(vm: &VirtualMachine) -> PyResult {
    Ok(vm.new_int(unistd::getpgrp().as_raw()))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn os_getsid(pid: u32, vm: &VirtualMachine) -> PyResult {
    match unistd::getsid(Some(Pid::from_raw(pid as i32))) {
        Ok(sid) => Ok(vm.new_int(sid.as_raw())),
        Err(err) => Err(convert_nix_error(vm, err)),
    }
}

#[cfg(unix)]
fn os_getuid(vm: &VirtualMachine) -> PyObjectRef {
    let uid = unistd::getuid().as_raw();
    vm.new_int(uid)
}

#[cfg(unix)]
fn os_geteuid(vm: &VirtualMachine) -> PyObjectRef {
    let euid = unistd::geteuid().as_raw();
    vm.new_int(euid)
}

#[cfg(unix)]
fn os_setgid(gid: u32, vm: &VirtualMachine) -> PyResult<()> {
    unistd::setgid(Gid::from_raw(gid)).map_err(|err| convert_nix_error(vm, err))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn os_setegid(egid: u32, vm: &VirtualMachine) -> PyResult<()> {
    unistd::setegid(Gid::from_raw(egid)).map_err(|err| convert_nix_error(vm, err))
}

#[cfg(unix)]
fn os_setpgid(pid: u32, pgid: u32, vm: &VirtualMachine) -> PyResult<()> {
    unistd::setpgid(Pid::from_raw(pid as i32), Pid::from_raw(pgid as i32))
        .map_err(|err| convert_nix_error(vm, err))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn os_setsid(vm: &VirtualMachine) -> PyResult<()> {
    unistd::setsid()
        .map(|_ok| ())
        .map_err(|err| convert_nix_error(vm, err))
}

#[cfg(unix)]
fn os_setuid(uid: u32, vm: &VirtualMachine) -> PyResult<()> {
    unistd::setuid(Uid::from_raw(uid)).map_err(|err| convert_nix_error(vm, err))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn os_seteuid(euid: u32, vm: &VirtualMachine) -> PyResult<()> {
    unistd::seteuid(Uid::from_raw(euid)).map_err(|err| convert_nix_error(vm, err))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn os_setreuid(ruid: u32, euid: u32, vm: &VirtualMachine) -> PyResult<()> {
    unistd::setuid(Uid::from_raw(ruid)).map_err(|err| convert_nix_error(vm, err))?;
    unistd::seteuid(Uid::from_raw(euid)).map_err(|err| convert_nix_error(vm, err))
}

// cfg from nix
#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "openbsd"
))]
fn os_setresuid(ruid: u32, euid: u32, suid: u32, vm: &VirtualMachine) -> PyResult<()> {
    unistd::setresuid(
        Uid::from_raw(ruid),
        Uid::from_raw(euid),
        Uid::from_raw(suid),
    )
    .map_err(|err| convert_nix_error(vm, err))
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn os_openpty(vm: &VirtualMachine) -> PyResult {
    match openpty(None, None) {
        Ok(r) => Ok(vm
            .ctx
            .new_tuple(vec![vm.new_int(r.master), vm.new_int(r.slave)])),
        Err(err) => Err(convert_nix_error(vm, err)),
    }
}

#[cfg(unix)]
pub fn os_ttyname(fd: i32, vm: &VirtualMachine) -> PyResult {
    use libc::ttyname;
    let name = unsafe { ttyname(fd) };
    if name.is_null() {
        Err(errno_err(vm))
    } else {
        let name = unsafe { ffi::CStr::from_ptr(name) }.to_str().unwrap();
        Ok(vm.ctx.new_str(name.to_owned()))
    }
}

fn os_urandom(size: usize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    let mut buf = vec![0u8; size];
    match getrandom::getrandom(&mut buf) {
        Ok(()) => Ok(buf),
        Err(e) => match e.raw_os_error() {
            Some(errno) => Err(convert_io_error(vm, io::Error::from_raw_os_error(errno))),
            None => Err(vm.new_os_error("Getting random failed".to_owned())),
        },
    }
}

#[cfg(any(target_os = "linux", target_os = "openbsd"))]
type ModeT = u32;

#[cfg(target_os = "redox")]
type ModeT = i32;

#[cfg(target_os = "macos")]
type ModeT = u16;

#[cfg(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "openbsd",
    target_os = "redox"
))]
fn os_umask(mask: ModeT, _vm: &VirtualMachine) -> PyResult<ModeT> {
    let ret_mask = unsafe { libc::umask(mask) };
    Ok(ret_mask)
}

#[pystruct_sequence(name = "os.uname_result")]
#[derive(Debug)]
#[cfg(unix)]
struct UnameResult {
    sysname: String,
    nodename: String,
    release: String,
    version: String,
    machine: String,
}

#[cfg(unix)]
impl UnameResult {
    fn into_obj(self, vm: &VirtualMachine) -> PyObjectRef {
        self.into_struct_sequence(vm, vm.class(MODULE_NAME, "uname_result"))
            .unwrap()
            .into_object()
    }
}

#[cfg(unix)]
fn os_uname(vm: &VirtualMachine) -> PyResult {
    let info = uname::uname().map_err(|err| convert_io_error(vm, err))?;
    Ok(UnameResult {
        sysname: info.sysname,
        nodename: info.nodename,
        release: info.release,
        version: info.version,
        machine: info.machine,
    }
    .into_obj(vm))
}

// this is basically what CPython has for Py_off_t; windows uses long long
// for offsets, other platforms just use off_t
#[cfg(not(windows))]
pub type Offset = libc::off_t;
#[cfg(windows)]
pub type Offset = libc::c_longlong;

#[cfg(windows)]
type InvalidParamHandler = extern "C" fn(
    *const libc::wchar_t,
    *const libc::wchar_t,
    *const libc::wchar_t,
    libc::c_uint,
    libc::uintptr_t,
);
#[cfg(windows)]
extern "C" {
    #[doc(hidden)]
    pub fn _set_thread_local_invalid_parameter_handler(
        pNew: InvalidParamHandler,
    ) -> InvalidParamHandler;
}

#[cfg(windows)]
#[doc(hidden)]
pub extern "C" fn silent_iph_handler(
    _: *const libc::wchar_t,
    _: *const libc::wchar_t,
    _: *const libc::wchar_t,
    _: libc::c_uint,
    _: libc::uintptr_t,
) {
}

#[macro_export]
macro_rules! suppress_iph {
    ($e:expr) => {{
        #[cfg(windows)]
        {
            let old = $crate::stdlib::os::_set_thread_local_invalid_parameter_handler(
                $crate::stdlib::os::silent_iph_handler,
            );
            let ret = $e;
            $crate::stdlib::os::_set_thread_local_invalid_parameter_handler(old);
            ret
        }
        #[cfg(not(windows))]
        {
            $e
        }
    }};
}

fn os_isatty(fd: i32) -> bool {
    unsafe { suppress_iph!(libc::isatty(fd)) != 0 }
}

fn os_lseek(fd: i32, position: Offset, how: i32, vm: &VirtualMachine) -> PyResult<Offset> {
    #[cfg(not(windows))]
    let res = unsafe { suppress_iph!(libc::lseek(fd, position, how)) };
    #[cfg(windows)]
    let res = unsafe {
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

fn os_link(src: PyPathLike, dst: PyPathLike, vm: &VirtualMachine) -> PyResult<()> {
    fs::hard_link(src.path, dst.path).map_err(|err| convert_io_error(vm, err))
}

fn os_utime(
    _path: PyPathLike,
    _time: OptionalArg<PyTupleRef>,
    _vm: &VirtualMachine,
) -> PyResult<()> {
    unimplemented!("utime")
}

#[cfg(unix)]
fn os_sync(_vm: &VirtualMachine) -> PyResult<()> {
    #[cfg(not(target_os = "redox"))]
    unsafe {
        libc::sync();
    }
    Ok(())
}

// cfg from nix
#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "openbsd"
))]
fn os_getresuid(vm: &VirtualMachine) -> PyResult<(u32, u32, u32)> {
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
#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "openbsd"
))]
fn os_getresgid(vm: &VirtualMachine) -> PyResult<(u32, u32, u32)> {
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
fn os_setresgid(rgid: u32, egid: u32, sgid: u32, vm: &VirtualMachine) -> PyResult<()> {
    unistd::setresgid(
        Gid::from_raw(rgid),
        Gid::from_raw(egid),
        Gid::from_raw(sgid),
    )
    .map_err(|err| convert_nix_error(vm, err))
}

// cfg from nix
#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "openbsd"
))]
fn os_setregid(rgid: u32, egid: u32, vm: &VirtualMachine) -> PyResult<()> {
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
fn os_initgroups(user_name: PyStringRef, gid: u32, vm: &VirtualMachine) -> PyResult<()> {
    let user = ffi::CString::new(user_name.as_str()).unwrap();
    let gid = Gid::from_raw(gid);
    unistd::initgroups(&user, gid).map_err(|err| convert_nix_error(vm, err))
}

// cfg from nix
#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "openbsd"
))]
fn os_setgroups(group_ids: PyIterable<u32>, vm: &VirtualMachine) -> PyResult<()> {
    let gids = group_ids
        .iter(vm)?
        .map(|entry| match entry {
            Ok(id) => Ok(unistd::Gid::from_raw(id)),
            Err(err) => Err(err),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ret = unistd::setgroups(&gids);
    ret.map_err(|err| convert_nix_error(vm, err))
}

#[cfg(unix)]
fn envp_from_dict(dict: PyDictRef, vm: &VirtualMachine) -> PyResult<Vec<ffi::CString>> {
    use std::os::unix::ffi::OsStringExt;
    dict.into_iter()
        .map(|(k, v)| {
            let k = PyPathLike::try_from_object(vm, k)?.path.into_vec();
            let v = PyPathLike::try_from_object(vm, v)?.path.into_vec();
            if k.contains(&0) {
                return Err(
                    vm.new_value_error("envp dict key cannot contain a nul byte".to_owned())
                );
            }
            if k.contains(&b'=') {
                return Err(
                    vm.new_value_error("envp dict key cannot contain a '=' character".to_owned())
                );
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
struct PosixSpawnArgs {
    #[pyarg(positional_only)]
    path: PyPathLike,
    #[pyarg(positional_only)]
    args: PyIterable<PyPathLike>,
    #[pyarg(positional_only)]
    env: PyMapping,
    #[pyarg(keyword_only, default = "None")]
    file_actions: Option<PyIterable<PyTupleRef>>,
    #[pyarg(keyword_only, default = "None")]
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
        use std::os::unix::ffi::OsStringExt;
        let path = ffi::CString::new(self.path.path.into_vec())
            .map_err(|_| vm.new_value_error("path should not have nul bytes".to_owned()))?;

        let mut file_actions = unsafe {
            let mut fa = std::mem::MaybeUninit::uninit();
            assert!(libc::posix_spawn_file_actions_init(fa.as_mut_ptr()) == 0);
            fa.assume_init()
        };
        if let Some(it) = self.file_actions {
            for action in it.iter(vm)? {
                let action = action?;
                let (id, args) = action.as_slice().split_first().ok_or_else(|| {
                    vm.new_type_error(
                        "Each file_actions element must be a non-empty tuple".to_owned(),
                    )
                })?;
                let id = i32::try_from_object(vm, id.clone())?;
                let id = PosixSpawnFileActionIdentifier::try_from(id)
                    .map_err(|_| vm.new_type_error("Unknown file_actions identifier".to_owned()))?;
                let args = PyFuncArgs::from(args.to_vec());
                let ret = match id {
                    PosixSpawnFileActionIdentifier::Open => {
                        let (fd, path, oflag, mode): (_, PyPathLike, _, _) = args.bind(vm)?;
                        let path = ffi::CString::new(path.path.into_vec()).map_err(|_| {
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
                        unsafe { libc::posix_spawn_file_actions_addclose(&mut file_actions, fd) }
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
            assert!(unsafe { libc::posix_spawnattr_setsigdefault(&mut attrp, set.as_ref()) } == 0);
        }

        let mut args: Vec<ffi::CString> = self
            .args
            .iter(vm)?
            .map(|res| {
                ffi::CString::new(res?.path.into_vec())
                    .map_err(|_| vm.new_value_error("path should not have nul bytes".to_owned()))
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
fn os_posix_spawn(args: PosixSpawnArgs, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
    args.spawn(false, vm)
}
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
fn os_posix_spawnp(args: PosixSpawnArgs, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
    args.spawn(true, vm)
}

#[cfg(unix)]
fn os_wifsignaled(status: i32) -> bool {
    unsafe { libc::WIFSIGNALED(status) }
}
#[cfg(unix)]
fn os_wifstopped(status: i32) -> bool {
    unsafe { libc::WIFSTOPPED(status) }
}
#[cfg(unix)]
fn os_wifexited(status: i32) -> bool {
    unsafe { libc::WIFEXITED(status) }
}
#[cfg(unix)]
fn os_wtermsig(status: i32) -> i32 {
    unsafe { libc::WTERMSIG(status) }
}
#[cfg(unix)]
fn os_wstopsig(status: i32) -> i32 {
    unsafe { libc::WSTOPSIG(status) }
}
#[cfg(unix)]
fn os_wexitstatus(status: i32) -> i32 {
    unsafe { libc::WEXITSTATUS(status) }
}

// TODO: os.wait[pid] for windows
#[cfg(unix)]
fn os_waitpid(pid: libc::pid_t, opt: i32, vm: &VirtualMachine) -> PyResult<(libc::pid_t, i32)> {
    let mut status = 0;
    let pid = unsafe { libc::waitpid(pid, &mut status, opt) };
    let pid = Errno::result(pid).map_err(|e| convert_nix_error(vm, e))?;
    Ok((pid, status))
}
#[cfg(unix)]
fn os_wait(vm: &VirtualMachine) -> PyResult<(libc::pid_t, i32)> {
    os_waitpid(-1, 0, vm)
}

fn os_kill(pid: i32, sig: isize, vm: &VirtualMachine) -> PyResult<()> {
    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(pid, sig as i32) };
        if ret == -1 {
            Err(errno_err(vm))
        } else {
            Ok(())
        }
    }
    #[cfg(windows)]
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
    #[cfg(not(any(unix, windows)))]
    {
        unimplemented!()
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let environ = _os_environ(vm);

    let scandir_iter = ctx.new_class("ScandirIter", ctx.object());
    ScandirIterator::extend_class(ctx, &scandir_iter);

    let dir_entry = py_class!(ctx, "DirEntry", ctx.object(), {
         "name" => ctx.new_readonly_getset("name", DirEntryRef::name),
         "path" => ctx.new_readonly_getset("path", DirEntryRef::path),
         "is_dir" => ctx.new_method(DirEntryRef::is_dir),
         "is_file" => ctx.new_method(DirEntryRef::is_file),
         "is_symlink" => ctx.new_method(DirEntryRef::is_symlink),
         "stat" => ctx.new_method(DirEntryRef::stat),
    });

    let stat_result = StatResult::make_class(ctx);

    struct SupportFunc<'a> {
        name: &'a str,
        func_obj: PyObjectRef,
        fd: Option<bool>,
        dir_fd: Option<bool>,
        follow_symlinks: Option<bool>,
    };
    impl<'a> SupportFunc<'a> {
        fn new<F, T, R, VM>(
            vm: &VirtualMachine,
            name: &'a str,
            func: F,
            fd: Option<bool>,
            dir_fd: Option<bool>,
            follow_symlinks: Option<bool>,
        ) -> Self
        where
            F: IntoPyNativeFunc<T, R, VM>,
        {
            let func_obj = vm.ctx.new_function(func);
            Self {
                name,
                func_obj,
                fd,
                dir_fd,
                follow_symlinks,
            }
        }
    }
    #[allow(unused_mut)]
    let mut support_funcs = vec![
        SupportFunc::new(vm, "open", os_open, None, Some(false), None),
        SupportFunc::new(vm, "access", os_access, Some(false), Some(false), None),
        SupportFunc::new(vm, "chdir", os_chdir, Some(false), None, None),
        // chflags Some, None Some
        // chown Some Some Some
        SupportFunc::new(vm, "listdir", os_listdir, Some(false), None, None),
        SupportFunc::new(vm, "mkdir", os_mkdir, Some(false), Some(false), None),
        // mkfifo Some Some None
        // mknod Some Some None
        // pathconf Some None None
        SupportFunc::new(vm, "readlink", os_readlink, Some(false), Some(false), None),
        SupportFunc::new(vm, "remove", os_remove, Some(false), Some(false), None),
        SupportFunc::new(vm, "rename", os_rename, Some(false), Some(false), None),
        SupportFunc::new(vm, "replace", os_rename, Some(false), Some(false), None), // TODO: Fix replace
        SupportFunc::new(vm, "rmdir", os_rmdir, Some(false), Some(false), None),
        SupportFunc::new(vm, "scandir", os_scandir, Some(false), None, None),
        SupportFunc::new(vm, "stat", os_stat, Some(false), Some(false), Some(false)),
        SupportFunc::new(vm, "fstat", os_stat, Some(false), Some(false), Some(false)),
        SupportFunc::new(vm, "symlink", os_symlink, None, Some(false), None),
        // truncate Some None None
        SupportFunc::new(vm, "unlink", os_remove, Some(false), Some(false), None),
        SupportFunc::new(vm, "utime", os_utime, Some(false), Some(false), Some(false)),
    ];
    #[cfg(unix)]
    support_funcs.extend(vec![
        SupportFunc::new(vm, "chmod", os_chmod, Some(false), Some(false), Some(false)),
        #[cfg(not(target_os = "redox"))]
        SupportFunc::new(vm, "chroot", os_chroot, Some(false), None, None),
        SupportFunc::new(vm, "umask", os_umask, Some(false), Some(false), Some(false)),
    ]);
    let supports_fd = PySet::default().into_ref(vm);
    let supports_dir_fd = PySet::default().into_ref(vm);
    let supports_follow_symlinks = PySet::default().into_ref(vm);

    let module = py_module!(vm, MODULE_NAME, {
        "close" => ctx.new_function(os_close),
        "error" => ctx.new_function(os_error),
        "fsync" => ctx.new_function(os_fsync),
        "read" => ctx.new_function(os_read),
        "write" => ctx.new_function(os_write),
        "mkdirs" => ctx.new_function(os_mkdirs),
        "putenv" => ctx.new_function(os_putenv),
        "unsetenv" => ctx.new_function(os_unsetenv),
        "environ" => environ,
        "ScandirIter" => scandir_iter,
        "DirEntry" => dir_entry,
        "stat_result" => stat_result,
        "lstat" => ctx.new_function(os_lstat),
        "getcwd" => ctx.new_function(os_getcwd),
        "chdir" => ctx.new_function(os_chdir),
        "fspath" => ctx.new_function(os_fspath),
        "getpid" => ctx.new_function(os_getpid),
        "cpu_count" => ctx.new_function(os_cpu_count),
        "_exit" => ctx.new_function(os_exit),
        "urandom" => ctx.new_function(os_urandom),
        "isatty" => ctx.new_function(os_isatty),
        "lseek" => ctx.new_function(os_lseek),
        "set_inheritable" => ctx.new_function(os_set_inheritable),
        "link" => ctx.new_function(os_link),
        "kill" => ctx.new_function(os_kill),

        "O_RDONLY" => ctx.new_int(libc::O_RDONLY),
        "O_WRONLY" => ctx.new_int(libc::O_WRONLY),
        "O_RDWR" => ctx.new_int(libc::O_RDWR),
        "O_APPEND" => ctx.new_int(libc::O_APPEND),
        "O_EXCL" => ctx.new_int(libc::O_EXCL),
        "O_CREAT" => ctx.new_int(libc::O_CREAT),
        "O_TRUNC" => ctx.new_int(libc::O_TRUNC),
        "F_OK" => ctx.new_int(0),
        "R_OK" => ctx.new_int(4),
        "W_OK" => ctx.new_int(2),
        "X_OK" => ctx.new_int(1),
        "SEEK_SET" => ctx.new_int(libc::SEEK_SET),
        "SEEK_CUR" => ctx.new_int(libc::SEEK_CUR),
        "SEEK_END" => ctx.new_int(libc::SEEK_END),
    });

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

    extend_module_platform_specific(&vm, &module);

    module
}

#[cfg(unix)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: &PyObjectRef) {
    let ctx = &vm.ctx;

    let uname_result = UnameResult::make_class(ctx);

    extend_module!(vm, module, {
        "chmod" => ctx.new_function(os_chmod),
        "get_inheritable" => ctx.new_function(os_get_inheritable), // TODO: windows
        "get_blocking" => ctx.new_function(os_get_blocking),
        "getppid" => ctx.new_function(os_getppid),
        "getgid" => ctx.new_function(os_getgid),
        "getegid" => ctx.new_function(os_getegid),
        "getpgid" => ctx.new_function(os_getpgid),
        "getuid" => ctx.new_function(os_getuid),
        "getpgrp" => ctx.new_function(os_getpgrp),
        "geteuid" => ctx.new_function(os_geteuid),
        "pipe" => ctx.new_function(os_pipe), //TODO: windows
        "set_blocking" => ctx.new_function(os_set_blocking),
        "setgid" => ctx.new_function(os_setgid),
        "setpgid" => ctx.new_function(os_setpgid),
        "setuid" => ctx.new_function(os_setuid),
        "sync" => ctx.new_function(os_sync),
        "system" => ctx.new_function(os_system),
        "ttyname" => ctx.new_function(os_ttyname),
        "uname" => ctx.new_function(os_uname),
        "uname_result" => uname_result,
        "wait" => ctx.new_function(os_wait),
        "waitpid" => ctx.new_function(os_waitpid),
        "WIFSIGNALED" => ctx.new_function(os_wifsignaled),
        "WIFSTOPPED" => ctx.new_function(os_wifstopped),
        "WIFEXITED" => ctx.new_function(os_wifexited),
        "WTERMSIG" => ctx.new_function(os_wtermsig),
        "WSTOPSIG" => ctx.new_function(os_wstopsig),
        "WEXITSTATUS" => ctx.new_function(os_wexitstatus),
        "WNOHANG" => ctx.new_int(libc::WNOHANG),
        "EX_OK" => ctx.new_int(exitcode::OK as i8),
        "EX_USAGE" => ctx.new_int(exitcode::USAGE as i8),
        "EX_DATAERR" => ctx.new_int(exitcode::DATAERR as i8),
        "EX_NOINPUT" => ctx.new_int(exitcode::NOINPUT as i8),
        "EX_NOUSER" => ctx.new_int(exitcode::NOUSER as i8),
        "EX_NOHOST" => ctx.new_int(exitcode::NOHOST as i8),
        "EX_UNAVAILABLE" => ctx.new_int(exitcode::UNAVAILABLE as i8),
        "EX_SOFTWARE" => ctx.new_int(exitcode::SOFTWARE as i8),
        "EX_OSERR" => ctx.new_int(exitcode::OSERR as i8),
        "EX_OSFILE" => ctx.new_int(exitcode::OSFILE as i8),
        "EX_CANTCREAT" => ctx.new_int(exitcode::CANTCREAT as i8),
        "EX_IOERR" => ctx.new_int(exitcode::IOERR as i8),
        "EX_TEMPFAIL" => ctx.new_int(exitcode::TEMPFAIL as i8),
        "EX_PROTOCOL" => ctx.new_int(exitcode::PROTOCOL as i8),
        "EX_NOPERM" => ctx.new_int(exitcode::NOPERM as i8),
        "EX_CONFIG" => ctx.new_int(exitcode::CONFIG as i8),
        "O_NONBLOCK" => ctx.new_int(libc::O_NONBLOCK),
        "O_CLOEXEC" => ctx.new_int(libc::O_CLOEXEC),
    });

    #[cfg(not(target_os = "redox"))]
    extend_module!(vm, module, {
        "chroot" => ctx.new_function(os_chroot),
        "getsid" => ctx.new_function(os_getsid),
        "setsid" => ctx.new_function(os_setsid),
        "setegid" => ctx.new_function(os_setegid),
        "seteuid" => ctx.new_function(os_seteuid),
        "setreuid" => ctx.new_function(os_setreuid),
        "openpty" => ctx.new_function(os_openpty),
        "O_DSYNC" => ctx.new_int(libc::O_DSYNC),
        "O_NDELAY" => ctx.new_int(libc::O_NDELAY),
        "O_NOCTTY" => ctx.new_int(libc::O_NOCTTY),
    });

    // cfg taken from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    extend_module!(vm, module, {
        "setresuid" => ctx.new_function(os_setresuid),
        "getresuid" => ctx.new_function(os_getresuid),
        "getresgid" => ctx.new_function(os_getresgid),
        "setresgid" => ctx.new_function(os_setresgid),
        "setregid" => ctx.new_function(os_setregid),
        "initgroups" => ctx.new_function(os_initgroups),
        "setgroups" => ctx.new_function(os_setgroups),
    });

    // cfg taken from nix
    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        all(
            target_os = "linux",
            not(any(target_env = "musl", target_arch = "mips", target_arch = "mips64"))
        )
    ))]
    extend_module!(vm, module, {
        "SEEK_DATA" => ctx.new_int(unistd::Whence::SeekData as i8),
        "SEEK_HOLE" => ctx.new_int(unistd::Whence::SeekHole as i8)
    });
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
    extend_module!(vm, module, {
        "pipe2" => ctx.new_function(os_pipe2),
    });

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    extend_module!(vm, module, {
        "posix_spawn" => ctx.new_function(os_posix_spawn),
        "posix_spawnp" => ctx.new_function(os_posix_spawnp),
        "POSIX_SPAWN_OPEN" => ctx.new_int(i32::from(PosixSpawnFileActionIdentifier::Open)),
        "POSIX_SPAWN_CLOSE" => ctx.new_int(i32::from(PosixSpawnFileActionIdentifier::Close)),
        "POSIX_SPAWN_DUP2" => ctx.new_int(i32::from(PosixSpawnFileActionIdentifier::Dup2)),
    });
}

#[cfg(windows)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: &PyObjectRef) {
    let ctx = &vm.ctx;
    extend_module!(vm, module, {
        "O_BINARY" => ctx.new_int(libc::O_BINARY),
    });
}

#[cfg(not(any(unix, windows)))]
fn extend_module_platform_specific(_vm: &VirtualMachine, _module: &PyObjectRef) {}
