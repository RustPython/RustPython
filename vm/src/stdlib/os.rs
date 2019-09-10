use num_cpus;
use std::cell::RefCell;
use std::ffi;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Read, Write};
use std::time::{Duration, SystemTime};
use std::{env, fs};

#[cfg(unix)]
use nix::errno::Errno;
#[cfg(all(unix, not(target_os = "redox")))]
use nix::pty::openpty;
#[cfg(unix)]
use nix::unistd::{self, Gid, Pid, Uid, Whence};
use num_traits::cast::ToPrimitive;

use bitflags::bitflags;

use crate::function::{IntoPyNativeFunc, PyFuncArgs};
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objdict::PyDictRef;
use crate::obj::objint::{self, PyInt, PyIntRef};
use crate::obj::objiter;
use crate::obj::objset::PySet;
use crate::obj::objstr::{self, PyString, PyStringRef};
use crate::obj::objtype::{self, PyClassRef};
use crate::pyobject::{
    ItemProtocol, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryIntoRef, TypeProtocol,
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

pub fn os_close(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(fileno, Some(vm.ctx.int_type()))]);

    let raw_fileno = objint::get_value(&fileno);

    //The File type automatically closes when it goes out of scope.
    //To enable us to close these file descriptors (and hence prevent leaks)
    //we seek to create the relevant File and simply let it pass out of scope!
    rust_file(raw_fileno.to_i64().unwrap());

    Ok(vm.get_none())
}

bitflags! {
     pub struct FileCreationFlags: u32 {
        // https://elixir.bootlin.com/linux/v4.8/source/include/uapi/asm-generic/fcntl.h
        const O_RDONLY = 0o0000_0000;
        const O_WRONLY = 0o0000_0001;
        const O_RDWR = 0o0000_0002;
        const O_CREAT = 0o0000_0100;
        const O_EXCL = 0o0000_0200;
        const O_APPEND = 0o0000_2000;
        const O_NONBLOCK = 0o0000_4000;
    }
}

pub fn os_open(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (name, Some(vm.ctx.str_type())),
            (flags, Some(vm.ctx.int_type()))
        ],
        optional = [
            (_mode, Some(vm.ctx.int_type())),
            (dir_fd, Some(vm.ctx.int_type()))
        ]
    );

    let name = name.clone().downcast::<PyString>().unwrap();
    let dir_fd = if let Some(obj) = dir_fd {
        DirFd {
            dir_fd: Some(obj.clone().downcast::<PyInt>().unwrap()),
        }
    } else {
        DirFd::default()
    };
    let fname = &make_path(vm, name, &dir_fd).value;

    let flags = FileCreationFlags::from_bits(objint::get_value(flags).to_u32().unwrap())
        .ok_or_else(|| vm.new_value_error("Unsupported flag".to_string()))?;

    let mut options = &mut OpenOptions::new();

    if flags.contains(FileCreationFlags::O_WRONLY) {
        options = options.write(true);
    } else if flags.contains(FileCreationFlags::O_RDWR) {
        options = options.read(true).write(true);
    } else {
        options = options.read(true);
    }

    if flags.contains(FileCreationFlags::O_APPEND) {
        options = options.append(true);
    }

    if flags.contains(FileCreationFlags::O_CREAT) {
        if flags.contains(FileCreationFlags::O_EXCL) {
            options = options.create_new(true);
        } else {
            options = options.create(true);
        }
    }

    let handle = options
        .open(&fname)
        .map_err(|err| convert_io_error(vm, err))?;

    Ok(vm.ctx.new_int(raw_file_number(handle)))
}

pub fn convert_io_error(vm: &VirtualMachine, err: io::Error) -> PyObjectRef {
    let os_error = match err.kind() {
        ErrorKind::NotFound => {
            let exc_type = vm.ctx.exceptions.file_not_found_error.clone();
            vm.new_exception(exc_type, err.to_string())
        }
        ErrorKind::PermissionDenied => {
            let exc_type = vm.ctx.exceptions.permission_error.clone();
            vm.new_exception(exc_type, err.to_string())
        }
        ErrorKind::AlreadyExists => {
            let exc_type = vm.ctx.exceptions.file_exists_error.clone();
            vm.new_exception(exc_type, err.to_string())
        }
        _ => vm.new_os_error(err.to_string()),
    };
    if let Some(errno) = err.raw_os_error() {
        vm.set_attr(&os_error, "errno", vm.ctx.new_int(errno))
            .unwrap();
    }
    os_error
}

#[cfg(unix)]
pub fn convert_nix_error(vm: &VirtualMachine, err: nix::Error) -> PyObjectRef {
    let nix_error = match err {
        nix::Error::InvalidPath => {
            let exc_type = vm.ctx.exceptions.file_not_found_error.clone();
            vm.new_exception(exc_type, err.to_string())
        }
        nix::Error::InvalidUtf8 => {
            let exc_type = vm.ctx.exceptions.unicode_error.clone();
            vm.new_exception(exc_type, err.to_string())
        }
        nix::Error::UnsupportedOperation => {
            let exc_type = vm.ctx.exceptions.runtime_error.clone();
            vm.new_exception(exc_type, err.to_string())
        }
        nix::Error::Sys(errno) => {
            let exc_type = convert_nix_errno(vm, errno);
            vm.new_exception(exc_type, err.to_string())
        }
    };

    if let nix::Error::Sys(errno) = err {
        vm.set_attr(&nix_error, "errno", vm.ctx.new_int(errno as i32))
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
    loop {
        let ret = unsafe {
            libc::getgroups(
                groups.capacity() as c_int,
                groups.as_mut_ptr() as *mut gid_t,
            )
        };

        return Errno::result(ret).map(|s| {
            unsafe { groups.set_len(s as usize) };
            groups
        });
    }
}

#[cfg(any(target_os = "linux", target_os = "redox", target_os = "android"))]
fn getgroups() -> nix::Result<Vec<Gid>> {
    nix::unistd::getgroups()
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

fn os_error(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [],
        optional = [(message, Some(vm.ctx.str_type()))]
    );

    let msg = if let Some(val) = message {
        objstr::get_value(&val)
    } else {
        "".to_string()
    };

    Err(vm.new_os_error(msg))
}

fn os_fsync(fd: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
    let file = rust_file(fd.as_bigint().to_i64().unwrap());
    file.sync_all().map_err(|err| convert_io_error(vm, err))?;
    // Avoid closing the fd
    raw_file_number(file);
    Ok(())
}

fn os_read(fd: PyIntRef, n: PyIntRef, vm: &VirtualMachine) -> PyResult {
    let mut buffer = vec![0u8; n.as_bigint().to_usize().unwrap()];
    let mut file = rust_file(fd.as_bigint().to_i64().unwrap());
    file.read_exact(&mut buffer)
        .map_err(|err| convert_io_error(vm, err))?;

    // Avoid closing the fd
    raw_file_number(file);
    Ok(vm.ctx.new_bytes(buffer))
}

fn os_write(fd: PyIntRef, data: PyBytesRef, vm: &VirtualMachine) -> PyResult {
    let mut file = rust_file(fd.as_bigint().to_i64().unwrap());
    let written = file.write(&data).map_err(|err| convert_io_error(vm, err))?;

    // Avoid closing the fd
    raw_file_number(file);
    Ok(vm.ctx.new_int(written))
}

fn os_remove(path: PyStringRef, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult<()> {
    let path = make_path(vm, path, &dir_fd);
    fs::remove_file(&path.value).map_err(|err| convert_io_error(vm, err))
}

fn os_mkdir(path: PyStringRef, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult<()> {
    let path = make_path(vm, path, &dir_fd);
    fs::create_dir(&path.value).map_err(|err| convert_io_error(vm, err))
}

fn os_mkdirs(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    fs::create_dir_all(&path.value).map_err(|err| convert_io_error(vm, err))
}

fn os_rmdir(path: PyStringRef, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult<()> {
    let path = make_path(vm, path, &dir_fd);
    fs::remove_dir(&path.value).map_err(|err| convert_io_error(vm, err))
}

fn os_listdir(path: PyStringRef, vm: &VirtualMachine) -> PyResult {
    match fs::read_dir(&path.value) {
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

fn os_putenv(key: PyStringRef, value: PyStringRef, _vm: &VirtualMachine) {
    env::set_var(&key.value, &value.value)
}

fn os_unsetenv(key: PyStringRef, _vm: &VirtualMachine) {
    env::remove_var(&key.value)
}

fn _os_environ(vm: &VirtualMachine) -> PyDictRef {
    let environ = vm.ctx.new_dict();
    for (key, value) in env::vars() {
        environ.set_item(&key, vm.new_str(value), vm).unwrap();
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

    fn stat(
        self,
        dir_fd: DirFd,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<StatResult> {
        os_stat(self.path(vm).try_into_ref(vm)?, dir_fd, follow_symlinks, vm)
    }
}

#[pyclass]
#[derive(Debug)]
struct ScandirIterator {
    entries: RefCell<fs::ReadDir>,
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
        match self.entries.borrow_mut().next() {
            Some(entry) => match entry {
                Ok(entry) => Ok(DirEntry { entry }.into_ref(vm).into_object()),
                Err(s) => Err(convert_io_error(vm, s)),
            },
            None => Err(objiter::new_stop_iteration(vm)),
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

fn os_scandir(path: PyStringRef, vm: &VirtualMachine) -> PyResult {
    match fs::read_dir(&path.value) {
        Ok(iter) => Ok(ScandirIterator {
            entries: RefCell::new(iter),
        }
        .into_ref(vm)
        .into_object()),
        Err(s) => Err(convert_io_error(vm, s)),
    }
}

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
    st_ctime: f64,
    st_mtime: f64,
}

impl PyValue for StatResult {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_os", "stat_result")
    }
}

type StatResultRef = PyRef<StatResult>;

impl StatResultRef {
    fn st_mode(self, _vm: &VirtualMachine) -> u32 {
        self.st_mode
    }

    fn st_ino(self, _vm: &VirtualMachine) -> u64 {
        self.st_ino
    }

    fn st_dev(self, _vm: &VirtualMachine) -> u64 {
        self.st_dev
    }

    fn st_nlink(self, _vm: &VirtualMachine) -> u64 {
        self.st_nlink
    }

    fn st_uid(self, _vm: &VirtualMachine) -> u32 {
        self.st_uid
    }

    fn st_gid(self, _vm: &VirtualMachine) -> u32 {
        self.st_gid
    }

    fn st_size(self, _vm: &VirtualMachine) -> u64 {
        self.st_size
    }

    fn st_atime(self, _vm: &VirtualMachine) -> f64 {
        self.st_atime
    }

    fn st_ctime(self, _vm: &VirtualMachine) -> f64 {
        self.st_ctime
    }

    fn st_mtime(self, _vm: &VirtualMachine) -> f64 {
        self.st_mtime
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
macro_rules! os_unix_stat_inner {
    ( $path:expr, $follow_symlinks:expr, $vm:expr ) => {{
        #[allow(clippy::match_bool)]
        fn get_stats(path: &str, follow_symlinks: bool) -> io::Result<StatResult> {
            let meta = match follow_symlinks {
                true => fs::metadata(path)?,
                false => fs::symlink_metadata(path)?,
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
            })
        }

        get_stats(&$path.value, $follow_symlinks.follow_symlinks)
            .map_err(|err| convert_io_error($vm, err))
    }};
}

#[cfg(target_os = "linux")]
fn os_stat(
    path: PyStringRef,
    dir_fd: DirFd,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult<StatResult> {
    use std::os::linux::fs::MetadataExt;
    let path = make_path(vm, path, &dir_fd);
    os_unix_stat_inner!(path, follow_symlinks, vm)
}

#[cfg(target_os = "macos")]
fn os_stat(
    path: PyStringRef,
    dir_fd: DirFd,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult<StatResult> {
    use std::os::macos::fs::MetadataExt;
    let path = make_path(vm, path, &dir_fd);
    os_unix_stat_inner!(path, follow_symlinks, vm)
}

#[cfg(target_os = "android")]
fn os_stat(
    path: PyStringRef,
    dir_fd: DirFd,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult<StatResult> {
    use std::os::android::fs::MetadataExt;
    let path = make_path(vm, path, &dir_fd);
    os_unix_stat_inner!(path, follow_symlinks, vm)
}

#[cfg(target_os = "redox")]
fn os_stat(
    path: PyStringRef,
    dir_fd: DirFd,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult<StatResult> {
    use std::os::redox::fs::MetadataExt;
    let path = make_path(vm, path, &dir_fd);
    os_unix_stat_inner!(path, follow_symlinks, vm)
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
    path: PyStringRef,
    _dir_fd: DirFd, // TODO: error
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult<StatResult> {
    use std::os::windows::fs::MetadataExt;

    fn get_stats(path: &str, follow_symlinks: bool) -> io::Result<StatResult> {
        let meta = match follow_symlinks {
            true => fs::metadata(path)?,
            false => fs::symlink_metadata(path)?,
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
        })
    }

    get_stats(&path.value, follow_symlinks.follow_symlinks)
        .map_err(|s| vm.new_os_error(s.to_string()))
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    target_os = "redox",
    windows
)))]
fn os_stat(
    _path: PyStringRef,
    _dir_fd: DirFd,
    _follow_symlinks: FollowSymlinks,
    _vm: &VirtualMachine,
) -> PyResult<StatResult> {
    unimplemented!();
}

fn os_lstat(path: PyStringRef, dir_fd: DirFd, vm: &VirtualMachine) -> PyResult<StatResult> {
    os_stat(
        path,
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
    unix_fs::symlink(&src.value, &dst.value).map_err(|err| convert_io_error(vm, err))
}

#[cfg(windows)]
fn os_symlink(
    src: PyStringRef,
    dst: PyStringRef,
    _dir_fd: DirFd,
    vm: &VirtualMachine,
) -> PyResult<()> {
    use std::os::windows::fs as win_fs;
    let ret = match fs::metadata(&dst.value) {
        Ok(meta) => {
            if meta.is_file() {
                win_fs::symlink_file(&src.value, &dst.value)
            } else if meta.is_dir() {
                win_fs::symlink_dir(&src.value, &dst.value)
            } else {
                panic!("Uknown file type");
            }
        }
        Err(_) => win_fs::symlink_file(&src.value, &dst.value),
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
    env::set_current_dir(&path.value).map_err(|err| convert_io_error(vm, err))
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
        fs::metadata(&path.value)
    } else {
        fs::symlink_metadata(&path.value)
    };
    let meta = metadata.map_err(|err| convert_io_error(vm, err))?;
    let mut permissions = meta.permissions();
    permissions.set_mode(mode);
    fs::set_permissions(&path.value, permissions).map_err(|err| convert_io_error(vm, err))?;
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
    fs::rename(&src.value, &dst.value).map_err(|err| convert_io_error(vm, err))
}

fn os_getpid(vm: &VirtualMachine) -> PyObjectRef {
    let pid = std::process::id();
    vm.new_int(pid)
}

fn os_cpu_count(vm: &VirtualMachine) -> PyObjectRef {
    let cpu_count = num_cpus::get();
    vm.new_int(cpu_count)
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
fn os_getpgid(pid: PyIntRef, vm: &VirtualMachine) -> PyObjectRef {
    let pid = pid.as_bigint().to_u32().unwrap();

    match unistd::getpgid(Some(Pid::from_raw(pid as i32))) {
        Ok(pgid) => vm.new_int(pgid.as_raw()),
        Err(err) => convert_nix_error(vm, err),
    }
}

#[cfg(all(unix, not(target_os = "redox")))]
fn os_getsid(pid: PyIntRef, vm: &VirtualMachine) -> PyObjectRef {
    let pid = pid.as_bigint().to_u32().unwrap();

    match unistd::getsid(Some(Pid::from_raw(pid as i32))) {
        Ok(sid) => vm.new_int(sid.as_raw()),
        Err(err) => convert_nix_error(vm, err),
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
fn os_setgid(gid: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
    let gid = gid.as_bigint().to_u32().unwrap();

    unistd::setgid(Gid::from_raw(gid)).map_err(|err| convert_nix_error(vm, err))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn os_setegid(egid: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
    let egid = egid.as_bigint().to_u32().unwrap();

    unistd::setegid(Gid::from_raw(egid)).map_err(|err| convert_nix_error(vm, err))
}

#[cfg(unix)]
fn os_setpgid(pid: PyIntRef, pgid: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
    let pid = pid.as_bigint().to_u32().unwrap();
    let pgid = pgid.as_bigint().to_u32().unwrap();

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
fn os_setuid(uid: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
    let uid = uid.as_bigint().to_u32().unwrap();

    unistd::setuid(Uid::from_raw(uid)).map_err(|err| convert_nix_error(vm, err))
}

#[cfg(all(unix, not(target_os = "redox")))]
fn os_seteuid(euid: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
    let euid = euid.as_bigint().to_u32().unwrap();

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
pub fn os_ttyname(fd: PyIntRef, vm: &VirtualMachine) -> PyResult {
    use libc::ttyname;
    if let Some(fd) = fd.as_bigint().to_i32() {
        let name = unsafe { ttyname(fd) };
        if name.is_null() {
            Err(vm.new_os_error(io::Error::last_os_error().to_string()))
        } else {
            let name = unsafe { ffi::CStr::from_ptr(name) }.to_str().unwrap();
            Ok(vm.ctx.new_str(name.to_owned()))
        }
    } else {
        Err(vm.new_overflow_error("signed integer is greater than maximum".to_owned()))
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
         "is_dir" => ctx.new_rustfunc(DirEntryRef::is_dir),
         "is_file" => ctx.new_rustfunc(DirEntryRef::is_file),
         "is_symlink" => ctx.new_rustfunc(DirEntryRef::is_symlink),
         "stat" => ctx.new_rustfunc(DirEntryRef::stat),
    });

    let stat_result = py_class!(ctx, "stat_result", ctx.object(), {
         "st_mode" => ctx.new_property(StatResultRef::st_mode),
         "st_ino" => ctx.new_property(StatResultRef::st_ino),
         "st_dev" => ctx.new_property(StatResultRef::st_dev),
         "st_nlink" => ctx.new_property(StatResultRef::st_nlink),
         "st_uid" => ctx.new_property(StatResultRef::st_uid),
         "st_gid" => ctx.new_property(StatResultRef::st_gid),
         "st_size" => ctx.new_property(StatResultRef::st_size),
         "st_atime" => ctx.new_property(StatResultRef::st_atime),
         "st_ctime" => ctx.new_property(StatResultRef::st_ctime),
         "st_mtime" => ctx.new_property(StatResultRef::st_mtime),
    });

    struct SupportFunc<'a> {
        name: &'a str,
        func_obj: PyObjectRef,
        fd: Option<bool>,
        dir_fd: Option<bool>,
        follow_symlinks: Option<bool>,
    };
    impl<'a> SupportFunc<'a> {
        fn new<F, T, R>(
            vm: &VirtualMachine,
            name: &'a str,
            func: F,
            fd: Option<bool>,
            dir_fd: Option<bool>,
            follow_symlinks: Option<bool>,
        ) -> Self
        where
            F: IntoPyNativeFunc<T, R>,
        {
            let func_obj = vm.ctx.new_rustfunc(func);
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
        // chroot Some None None
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
        SupportFunc::new(vm, "symlink", os_symlink, None, Some(false), None),
        // truncate Some None None
        SupportFunc::new(vm, "unlink", os_remove, Some(false), Some(false), None),
        // utime Some Some Some
    ];
    #[cfg(unix)]
    support_funcs.extend(vec![SupportFunc::new(
        vm,
        "chmod",
        os_chmod,
        Some(false),
        Some(false),
        Some(false),
    )]);
    let supports_fd = PySet::default().into_ref(vm);
    let supports_dir_fd = PySet::default().into_ref(vm);
    let supports_follow_symlinks = PySet::default().into_ref(vm);

    let module = py_module!(vm, "_os", {
        "close" => ctx.new_rustfunc(os_close),
        "error" => ctx.new_rustfunc(os_error),
        "fsync" => ctx.new_rustfunc(os_fsync),
        "read" => ctx.new_rustfunc(os_read),
        "write" => ctx.new_rustfunc(os_write),
        "mkdirs" => ctx.new_rustfunc(os_mkdirs),
        "putenv" => ctx.new_rustfunc(os_putenv),
        "unsetenv" => ctx.new_rustfunc(os_unsetenv),
        "environ" => environ,
        "name" => ctx.new_str(os_name),
        "ScandirIter" => scandir_iter,
        "DirEntry" => dir_entry,
        "stat_result" => stat_result,
        "lstat" => ctx.new_rustfunc(os_lstat),
        "getcwd" => ctx.new_rustfunc(os_getcwd),
        "chdir" => ctx.new_rustfunc(os_chdir),
        "fspath" => ctx.new_rustfunc(os_fspath),
        "O_RDONLY" => ctx.new_int(FileCreationFlags::O_RDONLY.bits()),
        "O_WRONLY" => ctx.new_int(FileCreationFlags::O_WRONLY.bits()),
        "O_RDWR" => ctx.new_int(FileCreationFlags::O_RDWR.bits()),
        "O_NONBLOCK" => ctx.new_int(FileCreationFlags::O_NONBLOCK.bits()),
        "O_APPEND" => ctx.new_int(FileCreationFlags::O_APPEND.bits()),
        "O_EXCL" => ctx.new_int(FileCreationFlags::O_EXCL.bits()),
        "O_CREAT" => ctx.new_int(FileCreationFlags::O_CREAT.bits()),
        "F_OK" => ctx.new_int(AccessFlags::F_OK.bits()),
        "R_OK" => ctx.new_int(AccessFlags::R_OK.bits()),
        "W_OK" => ctx.new_int(AccessFlags::W_OK.bits()),
        "X_OK" => ctx.new_int(AccessFlags::X_OK.bits()),
        "getpid" => ctx.new_rustfunc(os_getpid),
        "cpu_count" => ctx.new_rustfunc(os_cpu_count)
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
        "getppid" => ctx.new_rustfunc(os_getppid),
        "getgid" => ctx.new_rustfunc(os_getgid),
        "getegid" => ctx.new_rustfunc(os_getegid),
        "getpgid" => ctx.new_rustfunc(os_getpgid),
        "getuid" => ctx.new_rustfunc(os_getuid),
        "geteuid" => ctx.new_rustfunc(os_geteuid),
        "setgid" => ctx.new_rustfunc(os_setgid),
        "setpgid" => ctx.new_rustfunc(os_setpgid),
        "setuid" => ctx.new_rustfunc(os_setuid),
        "access" => ctx.new_rustfunc(os_access),
        "chmod" => ctx.new_rustfunc(os_chmod),
        "ttyname" => ctx.new_rustfunc(os_ttyname),
        "SEEK_SET" => ctx.new_int(Whence::SeekSet as i8),
        "SEEK_CUR" => ctx.new_int(Whence::SeekCur as i8),
        "SEEK_END" => ctx.new_int(Whence::SeekEnd as i8)
    });

    #[cfg(not(target_os = "redox"))]
    extend_module!(vm, module, {
        "getsid" => ctx.new_rustfunc(os_getsid),
        "setsid" => ctx.new_rustfunc(os_setsid),
        "setegid" => ctx.new_rustfunc(os_setegid),
        "seteuid" => ctx.new_rustfunc(os_seteuid),
        "openpty" => ctx.new_rustfunc(os_openpty),
    });

    #[cfg(not(any(target_os = "macos", target_os = "android", target_os = "redox")))]
    extend_module!(vm, module, {
        "SEEK_DATA" => ctx.new_int(Whence::SeekData as i8),
        "SEEK_HOLE" => ctx.new_int(Whence::SeekHole as i8)
    });

    module
}

#[cfg(not(unix))]
fn extend_module_platform_specific(_vm: &VirtualMachine, module: PyObjectRef) -> PyObjectRef {
    module
}
