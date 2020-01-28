use num_cpus;
use std::cell::{Cell, RefCell};
use std::ffi;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
use std::time::{Duration, SystemTime};
use std::{env, fs};

use bitflags::bitflags;
#[cfg(unix)]
use exitcode;
#[cfg(unix)]
use nix::errno::Errno;
#[cfg(all(unix, not(target_os = "redox")))]
use nix::pty::openpty;
#[cfg(unix)]
use nix::unistd::{self, Gid, Pid, Uid, Whence};
#[cfg(unix)]
use std::os::unix::io::RawFd;

use super::errno::errors;
use crate::exceptions::PyBaseExceptionRef;
use crate::function::{IntoPyNativeFunc, OptionalArg, PyFuncArgs};
use crate::obj::objbyteinner::PyBytesLike;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objdict::PyDictRef;
use crate::obj::objint::PyIntRef;
use crate::obj::objiter;
use crate::obj::objset::PySet;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::{self, PyClassRef};
use crate::pyobject::{
    Either, ItemProtocol, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryIntoRef,
    TypeProtocol,
};
use crate::vm::VirtualMachine;

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
    use std::os::windows::io::FromRawHandle;

    //This seems to work as expected but further testing is required.
    unsafe { File::from_raw_handle(raw_fileno as *mut ffi::c_void) }
}

#[cfg(all(not(unix), not(windows)))]
pub fn rust_file(raw_fileno: i64) -> File {
    unimplemented!();
}

#[cfg(all(not(unix), not(windows)))]
pub fn raw_file_number(handle: File) -> i64 {
    unimplemented!();
}

fn make_path(_vm: &VirtualMachine, path: PyStringRef, dir_fd: &DirFd) -> PyStringRef {
    if dir_fd.dir_fd.is_some() {
        unimplemented!();
    } else {
        path
    }
}

fn os_close(fileno: i64, _vm: &VirtualMachine) {
    //The File type automatically closes when it goes out of scope.
    //To enable us to close these file descriptors (and hence prevent leaks)
    //we seek to create the relevant File and simply let it pass out of scope!
    rust_file(fileno);
}

#[cfg(unix)]
type OpenFlags = i32;
#[cfg(windows)]
type OpenFlags = u32;

#[cfg(any(unix, windows))]
pub fn os_open(
    name: PyStringRef,
    flags: OpenFlags,
    _mode: OptionalArg<PyIntRef>,
    dir_fd: OptionalArg<PyIntRef>,
    vm: &VirtualMachine,
) -> PyResult<i64> {
    let dir_fd = DirFd {
        dir_fd: dir_fd.into_option(),
    };
    let fname = make_path(vm, name, &dir_fd);

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

    options.custom_flags(flags);
    let handle = options
        .open(fname.as_str())
        .map_err(|err| convert_io_error(vm, err))?;

    Ok(raw_file_number(handle))
}

#[cfg(all(not(unix), not(windows)))]
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
        nix::Error::UnsupportedOperation => {
            let exc_type = vm.ctx.exceptions.runtime_error.clone();
            vm.new_exception_msg(exc_type, err.to_string())
        }
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

#[cfg(any(target_os = "linux", target_os = "android"))]
use nix::unistd::getgroups;

#[cfg(target_os = "redox")]
fn getgroups() -> nix::Result<Vec<Gid>> {
    unimplemented!("redox getgroups")
}

#[cfg(unix)]
fn os_access(path: PyStringRef, mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
    use std::os::unix::fs::MetadataExt;

    let path = path.as_str();

    let flags = AccessFlags::from_bits(mode).ok_or_else(|| {
        vm.new_value_error(
            "One of the flags is wrong, there are only 4 possibilities F_OK, R_OK, W_OK and X_OK"
                .to_string(),
        )
    })?;

    let metadata = fs::metadata(path);

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

fn os_error(message: OptionalArg<PyStringRef>, vm: &VirtualMachine) -> PyResult {
    let msg = message.map_or("".to_string(), |msg| msg.as_str().to_string());

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

fn os_remove(path: PyStringRef, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult<()> {
    let path = make_path(vm, path, &dir_fd);
    fs::remove_file(path.as_str()).map_err(|err| convert_io_error(vm, err))
}

fn os_mkdir(
    path: PyStringRef,
    _mode: OptionalArg<PyIntRef>,
    dir_fd: DirFd,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let path = make_path(vm, path, &dir_fd);
    fs::create_dir(path.as_str()).map_err(|err| convert_io_error(vm, err))
}

fn os_mkdirs(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    fs::create_dir_all(path.as_str()).map_err(|err| convert_io_error(vm, err))
}

fn os_rmdir(path: PyStringRef, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult<()> {
    let path = make_path(vm, path, &dir_fd);
    fs::remove_dir(path.as_str()).map_err(|err| convert_io_error(vm, err))
}

fn os_listdir(path: PyStringRef, vm: &VirtualMachine) -> PyResult {
    match fs::read_dir(path.as_str()) {
        Ok(iter) => {
            let res: PyResult<Vec<PyObjectRef>> = iter
                .map(|entry| match entry {
                    Ok(path) => Ok(vm.ctx.new_str(path.file_name().into_string().unwrap())),
                    Err(s) => Err(convert_io_error(vm, s)),
                })
                .collect();
            Ok(vm.ctx.new_list(res?))
        }
        Err(s) => Err(vm.new_os_error(s.to_string())),
    }
}

fn bytes_as_osstr<'a>(b: &'a [u8], vm: &VirtualMachine) -> PyResult<&'a ffi::OsStr> {
    let os_str = {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Some(ffi::OsStr::from_bytes(b))
        }
        #[cfg(windows)]
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

fn os_readlink(path: PyStringRef, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult {
    let path = make_path(vm, path, &dir_fd);
    let path = fs::read_link(path.as_str()).map_err(|err| convert_io_error(vm, err))?;
    let path = path.into_os_string().into_string().map_err(|_osstr| {
        vm.new_unicode_decode_error("Can't convert OS path to valid UTF-8 string".into())
    })?;
    Ok(vm.ctx.new_str(path))
}

#[derive(Debug)]
struct DirEntry {
    entry: fs::DirEntry,
}

type DirEntryRef = PyRef<DirEntry>;

impl PyValue for DirEntry {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_os", "DirEntry")
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
    fn name(self, _vm: &VirtualMachine) -> String {
        self.entry.file_name().into_string().unwrap()
    }

    fn path(self, _vm: &VirtualMachine) -> String {
        self.entry.path().to_str().unwrap().to_string()
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
            Either::A(self.path(vm).try_into_ref(vm)?),
            dir_fd,
            follow_symlinks,
            vm,
        )
    }
}

#[pyclass]
#[derive(Debug)]
struct ScandirIterator {
    entries: RefCell<fs::ReadDir>,
    exhausted: Cell<bool>,
}

impl PyValue for ScandirIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_os", "ScandirIter")
    }
}

#[pyimpl]
impl ScandirIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if self.exhausted.get() {
            return Err(objiter::new_stop_iteration(vm));
        }

        match self.entries.borrow_mut().next() {
            Some(entry) => match entry {
                Ok(entry) => Ok(DirEntry { entry }.into_ref(vm).into_object()),
                Err(s) => Err(convert_io_error(vm, s)),
            },
            None => {
                self.exhausted.set(true);
                Err(objiter::new_stop_iteration(vm))
            }
        }
    }

    #[pymethod]
    fn close(&self, _vm: &VirtualMachine) {
        self.exhausted.set(true);
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__enter__")]
    fn enter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__exit__")]
    fn exit(zelf: PyRef<Self>, _args: PyFuncArgs, vm: &VirtualMachine) {
        zelf.close(vm)
    }
}

fn os_scandir(path: OptionalArg<PyStringRef>, vm: &VirtualMachine) -> PyResult {
    let path = match path {
        OptionalArg::Present(ref path) => path.as_str(),
        OptionalArg::Missing => ".",
    };

    match fs::read_dir(path) {
        Ok(iter) => Ok(ScandirIterator {
            entries: RefCell::new(iter),
            exhausted: Cell::new(false),
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
        self.into_struct_sequence(vm, vm.class("_os", "stat_result"))
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
    file: Either<PyStringRef, i64>,
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
    #[cfg(target_os = "redox")]
    use std::os::redox::fs::MetadataExt;

    let get_stats = move || -> io::Result<PyObjectRef> {
        let meta = match file {
            Either::A(path) => {
                let path = make_path(vm, path, &dir_fd);
                let path = path.as_str();
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
    file: Either<PyStringRef, i64>,
    _dir_fd: DirFd, // TODO: error
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult {
    use std::os::windows::fs::MetadataExt;

    let get_stats = move || -> io::Result<PyObjectRef> {
        let meta = match file {
            Either::A(path) => match follow_symlinks.follow_symlinks {
                true => fs::metadata(path.as_str())?,
                false => fs::symlink_metadata(path.as_str())?,
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
    windows
)))]
fn os_stat(
    _file: Either<PyStringRef, i64>,
    _dir_fd: DirFd,
    _follow_symlinks: FollowSymlinks,
    _vm: &VirtualMachine,
) -> PyResult {
    unimplemented!();
}

fn os_lstat(file: Either<PyStringRef, i64>, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult {
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
    src: PyStringRef,
    dst: PyStringRef,
    dir_fd: DirFd,
    vm: &VirtualMachine,
) -> PyResult<()> {
    use std::os::unix::fs as unix_fs;
    let dst = make_path(vm, dst, &dir_fd);
    unix_fs::symlink(src.as_str(), dst.as_str()).map_err(|err| convert_io_error(vm, err))
}

#[cfg(windows)]
fn os_symlink(
    src: PyStringRef,
    dst: PyStringRef,
    _dir_fd: DirFd,
    vm: &VirtualMachine,
) -> PyResult<()> {
    use std::os::windows::fs as win_fs;
    let meta = fs::metadata(src.as_str()).map_err(|err| convert_io_error(vm, err))?;
    let ret = if meta.is_file() {
        win_fs::symlink_file(src.as_str(), dst.as_str())
    } else if meta.is_dir() {
        win_fs::symlink_dir(src.as_str(), dst.as_str())
    } else {
        panic!("Uknown file type");
    };
    ret.map_err(|err| convert_io_error(vm, err))
}

#[cfg(all(not(unix), not(windows)))]
fn os_symlink(
    src: PyStringRef,
    dst: PyStringRef,
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
        .to_string())
}

fn os_chdir(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    env::set_current_dir(path.as_str()).map_err(|err| convert_io_error(vm, err))
}

#[cfg(unix)]
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

#[cfg(unix)]
fn os_set_inheritable(fd: RawFd, inheritable: bool, vm: &VirtualMachine) -> PyResult<()> {
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
    os_set_inheritable(rfd, false, vm)
        .and_then(|_| os_set_inheritable(wfd, false, vm))
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
fn os_system(command: PyStringRef, _vm: &VirtualMachine) -> PyResult<i32> {
    use libc::system;
    use std::ffi::CString;

    let rstr = command.as_str();
    let cstr = CString::new(rstr).unwrap();
    let x = unsafe { system(cstr.as_ptr()) };
    Ok(x)
}

#[cfg(unix)]
fn os_chmod(
    path: PyStringRef,
    dir_fd: DirFd,
    mode: u32,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = make_path(vm, path, &dir_fd);
    let metadata = if follow_symlinks.follow_symlinks {
        fs::metadata(path.as_str())
    } else {
        fs::symlink_metadata(path.as_str())
    };
    let meta = metadata.map_err(|err| convert_io_error(vm, err))?;
    let mut permissions = meta.permissions();
    permissions.set_mode(mode);
    fs::set_permissions(path.as_str(), permissions).map_err(|err| convert_io_error(vm, err))?;
    Ok(())
}

fn os_fspath(path: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    if objtype::issubclass(&path.class(), &vm.ctx.str_type())
        || objtype::issubclass(&path.class(), &vm.ctx.bytes_type())
    {
        Ok(path)
    } else {
        Err(vm.new_type_error(format!(
            "expected str or bytes object, not {}",
            path.class()
        )))
    }
}

fn os_rename(src: PyStringRef, dst: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    fs::rename(src.as_str(), dst.as_str()).map_err(|err| convert_io_error(vm, err))
}

fn os_getpid(vm: &VirtualMachine) -> PyObjectRef {
    let pid = std::process::id();
    vm.new_int(pid)
}

fn os_cpu_count(vm: &VirtualMachine) -> PyObjectRef {
    let cpu_count = num_cpus::get();
    vm.new_int(cpu_count)
}

fn os_exit(code: i32, _vm: &VirtualMachine) {
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
            None => Err(vm.new_os_error("Getting random failed".to_string())),
        },
    }
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
    fn _set_thread_local_invalid_parameter_handler(
        pNew: InvalidParamHandler,
    ) -> InvalidParamHandler;
}

#[cfg(windows)]
extern "C" fn silent_iph_handler(
    _: *const libc::wchar_t,
    _: *const libc::wchar_t,
    _: *const libc::wchar_t,
    _: libc::c_uint,
    _: libc::uintptr_t,
) {
}

macro_rules! suppress_iph {
    ($e:expr) => {{
        #[cfg(windows)]
        {
            let old = _set_thread_local_invalid_parameter_handler(silent_iph_handler);
            let ret = $e;
            _set_thread_local_invalid_parameter_handler(old);
            ret
        }
        #[cfg(not(windows))]
        {
            $e
        }
    }};
}

fn os_isatty(fd: i32, _vm: &VirtualMachine) -> bool {
    unsafe { suppress_iph!(libc::isatty(fd)) != 0 }
}

fn os_lseek(fd: i32, position: Offset, how: i32, vm: &VirtualMachine) -> PyResult<Offset> {
    #[cfg(not(windows))]
    use libc::lseek;
    #[cfg(windows)]
    use libc::lseek64 as lseek;
    let res = unsafe { suppress_iph!(lseek(fd, position, how)) };
    if res < 0 {
        Err(errno_err(vm))
    } else {
        Ok(res)
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let os_name = if cfg!(windows) {
        "nt".to_string()
    } else {
        "posix".to_string()
    };

    let environ = _os_environ(vm);

    let scandir_iter = ctx.new_class("ScandirIter", ctx.object());
    ScandirIterator::extend_class(ctx, &scandir_iter);

    let dir_entry = py_class!(ctx, "DirEntry", ctx.object(), {
         "name" => ctx.new_property(DirEntryRef::name),
         "path" => ctx.new_property(DirEntryRef::path),
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
        // access Some Some None
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
        // utime Some Some Some
    ];
    #[cfg(unix)]
    support_funcs.extend(vec![
        SupportFunc::new(vm, "chmod", os_chmod, Some(false), Some(false), Some(false)),
        SupportFunc::new(vm, "chroot", os_chroot, Some(false), None, None),
    ]);
    let supports_fd = PySet::default().into_ref(vm);
    let supports_dir_fd = PySet::default().into_ref(vm);
    let supports_follow_symlinks = PySet::default().into_ref(vm);

    let module = py_module!(vm, "_os", {
        "close" => ctx.new_function(os_close),
        "error" => ctx.new_function(os_error),
        "fsync" => ctx.new_function(os_fsync),
        "read" => ctx.new_function(os_read),
        "write" => ctx.new_function(os_write),
        "mkdirs" => ctx.new_function(os_mkdirs),
        "putenv" => ctx.new_function(os_putenv),
        "unsetenv" => ctx.new_function(os_unsetenv),
        "environ" => environ,
        "name" => ctx.new_str(os_name),
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

    extend_module_platform_specific(&vm, module)
}

#[cfg(unix)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: PyObjectRef) -> PyObjectRef {
    let ctx = &vm.ctx;
    extend_module!(vm, module, {
        "access" => ctx.new_function(os_access),
        "chmod" => ctx.new_function(os_chmod),
        "chroot" => ctx.new_function(os_chroot),
        "get_inheritable" => ctx.new_function(os_get_inheritable), // TODO: windows
        "get_blocking" => ctx.new_function(os_get_blocking),
        "getppid" => ctx.new_function(os_getppid),
        "getgid" => ctx.new_function(os_getgid),
        "getegid" => ctx.new_function(os_getegid),
        "getpgid" => ctx.new_function(os_getpgid),
        "getuid" => ctx.new_function(os_getuid),
        "geteuid" => ctx.new_function(os_geteuid),
        "pipe" => ctx.new_function(os_pipe), //TODO: windows
        "set_inheritable" => ctx.new_function(os_set_inheritable), // TODO: windows
        "set_blocking" => ctx.new_function(os_set_blocking),
        "setgid" => ctx.new_function(os_setgid),
        "setpgid" => ctx.new_function(os_setpgid),
        "setuid" => ctx.new_function(os_setuid),
        "system" => ctx.new_function(os_system),
        "ttyname" => ctx.new_function(os_ttyname),
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
        "getsid" => ctx.new_function(os_getsid),
        "setsid" => ctx.new_function(os_setsid),
        "setegid" => ctx.new_function(os_setegid),
        "seteuid" => ctx.new_function(os_seteuid),
        "openpty" => ctx.new_function(os_openpty),
        "O_DSYNC" => ctx.new_int(libc::O_DSYNC),
        "O_NDELAY" => ctx.new_int(libc::O_NDELAY),
        "O_NOCTTY" => ctx.new_int(libc::O_NOCTTY),
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
        "SEEK_DATA" => ctx.new_int(Whence::SeekData as i8),
        "SEEK_HOLE" => ctx.new_int(Whence::SeekHole as i8)
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

    module
}

#[cfg(not(unix))]
fn extend_module_platform_specific(_vm: &VirtualMachine, module: PyObjectRef) -> PyObjectRef {
    module
}
