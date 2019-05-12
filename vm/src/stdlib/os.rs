use std::cell::RefCell;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Read, Write};
use std::time::{Duration, SystemTime};
use std::{env, fs};

use num_traits::cast::ToPrimitive;

use crate::function::PyFuncArgs;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objdict::PyDictRef;
use crate::obj::objint;
use crate::obj::objint::PyIntRef;
use crate::obj::objiter;
use crate::obj::objstr;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    ItemProtocol, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryIntoRef,
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
    use std::ffi::c_void;
    use std::os::windows::io::FromRawHandle;

    //This seems to work as expected but further testing is required.
    unsafe { File::from_raw_handle(raw_fileno as *mut c_void) }
}

#[cfg(all(not(unix), not(windows)))]
pub fn rust_file(raw_fileno: i64) -> File {
    unimplemented!();
}

#[cfg(all(not(unix), not(windows)))]
pub fn raw_file_number(handle: File) -> i64 {
    unimplemented!();
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

pub fn os_open(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (name, Some(vm.ctx.str_type())),
            (mode, Some(vm.ctx.int_type()))
        ]
    );

    let fname = objstr::get_value(&name);

    let handle = match objint::get_value(mode).to_u16().unwrap() {
        0 => OpenOptions::new().read(true).open(&fname),
        1 => OpenOptions::new().write(true).open(&fname),
        2 => OpenOptions::new().read(true).write(true).open(&fname),
        512 => OpenOptions::new().write(true).create(true).open(&fname),
        _ => OpenOptions::new().read(true).open(&fname),
    }
    .map_err(|err| match err.kind() {
        ErrorKind::NotFound => {
            let exc_type = vm.ctx.exceptions.file_not_found_error.clone();
            vm.new_exception(exc_type, format!("No such file or directory: {}", &fname))
        }
        ErrorKind::PermissionDenied => {
            let exc_type = vm.ctx.exceptions.permission_error.clone();
            vm.new_exception(exc_type, format!("Permission denied: {}", &fname))
        }
        _ => vm.new_value_error("Unhandled file IO error".to_string()),
    })?;

    Ok(vm.ctx.new_int(raw_file_number(handle)))
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
    file.sync_all()
        .map_err(|s| vm.new_os_error(s.to_string()))?;
    // Avoid closing the fd
    raw_file_number(file);
    Ok(())
}

fn os_read(fd: PyIntRef, n: PyIntRef, vm: &VirtualMachine) -> PyResult {
    let mut buffer = vec![0u8; n.as_bigint().to_usize().unwrap()];
    let mut file = rust_file(fd.as_bigint().to_i64().unwrap());
    match file.read_exact(&mut buffer) {
        Ok(_) => (),
        Err(s) => return Err(vm.new_os_error(s.to_string())),
    };

    // Avoid closing the fd
    raw_file_number(file);
    Ok(vm.ctx.new_bytes(buffer))
}

fn os_write(fd: PyIntRef, data: PyBytesRef, vm: &VirtualMachine) -> PyResult {
    let mut file = rust_file(fd.as_bigint().to_i64().unwrap());
    let written = match file.write(&data) {
        Ok(written) => written,
        Err(s) => return Err(vm.new_os_error(s.to_string())),
    };

    // Avoid closing the fd
    raw_file_number(file);
    Ok(vm.ctx.new_int(written))
}

fn os_remove(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    fs::remove_file(&path.value).map_err(|s| vm.new_os_error(s.to_string()))
}

fn os_mkdir(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    fs::create_dir(&path.value).map_err(|s| vm.new_os_error(s.to_string()))
}

fn os_mkdirs(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    fs::create_dir_all(&path.value).map_err(|s| vm.new_os_error(s.to_string()))
}

fn os_rmdir(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    fs::remove_dir(&path.value).map_err(|s| vm.new_os_error(s.to_string()))
}

fn os_listdir(path: PyStringRef, vm: &VirtualMachine) -> PyResult {
    match fs::read_dir(&path.value) {
        Ok(iter) => {
            let res: PyResult<Vec<PyObjectRef>> = iter
                .map(|entry| match entry {
                    Ok(path) => Ok(vm.ctx.new_str(path.file_name().into_string().unwrap())),
                    Err(s) => Err(vm.new_os_error(s.to_string())),
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

#[derive(Debug)]
struct DirEntry {
    entry: fs::DirEntry,
}

type DirEntryRef = PyRef<DirEntry>;

impl PyValue for DirEntry {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("os", "DirEntry")
    }
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

    fn perform_on_metadata(
        self,
        follow_symlinks: FollowSymlinks,
        action: &Fn(fs::Metadata) -> bool,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let metadata = match follow_symlinks.follow_symlinks {
            true => fs::metadata(self.entry.path()),
            false => fs::symlink_metadata(self.entry.path()),
        };
        let meta = metadata.map_err(|s| vm.new_os_error(s.to_string()))?;
        Ok(action(meta))
    }

    fn is_dir(self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<bool> {
        self.perform_on_metadata(
            follow_symlinks,
            &|meta: fs::Metadata| -> bool { meta.is_dir() },
            vm,
        )
    }

    fn is_file(self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<bool> {
        self.perform_on_metadata(
            follow_symlinks,
            &|meta: fs::Metadata| -> bool { meta.is_file() },
            vm,
        )
    }

    fn is_symlink(self, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(self
            .entry
            .file_type()
            .map_err(|s| vm.new_os_error(s.to_string()))?
            .is_symlink())
    }

    fn stat(self, follow_symlinks: FollowSymlinks, vm: &VirtualMachine) -> PyResult<StatResult> {
        os_stat(self.path(vm).try_into_ref(vm)?, follow_symlinks, vm)
    }
}

#[pyclass]
#[derive(Debug)]
struct ScandirIterator {
    entries: RefCell<fs::ReadDir>,
}

impl PyValue for ScandirIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("os", "ScandirIter")
    }
}

#[pyimpl]
impl ScandirIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        match self.entries.borrow_mut().next() {
            Some(entry) => match entry {
                Ok(entry) => Ok(DirEntry { entry }.into_ref(vm).into_object()),
                Err(s) => Err(vm.new_os_error(s.to_string())),
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
        Err(s) => Err(vm.new_os_error(s.to_string())),
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
        vm.class("os", "stat_result")
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
    (duration.as_secs() as f64) + (duration.subsec_nanos() as f64) / (1_000_000_000 as f64)
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
            .map_err(|s| $vm.new_os_error(s.to_string()))
    }};
}

#[cfg(target_os = "linux")]
fn os_stat(
    path: PyStringRef,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult<StatResult> {
    use std::os::linux::fs::MetadataExt;
    os_unix_stat_inner!(path, follow_symlinks, vm)
}

#[cfg(target_os = "macos")]
fn os_stat(
    path: PyStringRef,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult<StatResult> {
    use std::os::macos::fs::MetadataExt;
    os_unix_stat_inner!(path, follow_symlinks, vm)
}

#[cfg(target_os = "android")]
fn os_stat(
    path: PyStringRef,
    follow_symlinks: FollowSymlinks,
    vm: &VirtualMachine,
) -> PyResult<StatResult> {
    use std::os::android::fs::MetadataExt;
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
    windows
)))]
fn os_stat(path: PyStringRef, vm: &VirtualMachine) -> PyResult {
    unimplemented!();
}

#[cfg(unix)]
fn os_symlink(src: PyStringRef, dst: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    use std::os::unix::fs as unix_fs;
    unix_fs::symlink(&src.value, &dst.value).map_err(|s| vm.new_os_error(s.to_string()))
}

#[cfg(windows)]
fn os_symlink(src: PyStringRef, dst: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
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
    ret.map_err(|s| vm.new_os_error(s.to_string()))
}

#[cfg(all(not(unix), not(windows)))]
fn os_symlink(src: PyStringRef, dst: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    unimplemented!();
}

fn os_getcwd(vm: &VirtualMachine) -> PyResult<String> {
    Ok(env::current_dir()
        .map_err(|s| vm.new_os_error(s.to_string()))?
        .as_path()
        .to_str()
        .unwrap()
        .to_string())
}

fn os_chdir(path: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    env::set_current_dir(&path.value).map_err(|s| vm.new_os_error(s.to_string()))
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

    py_module!(vm, "_os", {
        "open" => ctx.new_rustfunc(os_open),
        "close" => ctx.new_rustfunc(os_close),
        "error" => ctx.new_rustfunc(os_error),
        "fsync" => ctx.new_rustfunc(os_fsync),
        "read" => ctx.new_rustfunc(os_read),
        "write" => ctx.new_rustfunc(os_write),
        "remove" => ctx.new_rustfunc(os_remove),
        "unlink" => ctx.new_rustfunc(os_remove),
        "mkdir" => ctx.new_rustfunc(os_mkdir),
        "mkdirs" => ctx.new_rustfunc(os_mkdirs),
        "rmdir" => ctx.new_rustfunc(os_rmdir),
        "listdir" => ctx.new_rustfunc(os_listdir),
        "putenv" => ctx.new_rustfunc(os_putenv),
        "unsetenv" => ctx.new_rustfunc(os_unsetenv),
        "environ" => environ,
        "name" => ctx.new_str(os_name),
        "scandir" => ctx.new_rustfunc(os_scandir),
        "ScandirIter" => scandir_iter,
        "DirEntry" => dir_entry,
        "stat_result" => stat_result,
        "stat" => ctx.new_rustfunc(os_stat),
        "symlink" => ctx.new_rustfunc(os_symlink),
        "getcwd" => ctx.new_rustfunc(os_getcwd),
        "chdir" => ctx.new_rustfunc(os_chdir),
        "O_RDONLY" => ctx.new_int(0),
        "O_WRONLY" => ctx.new_int(1),
        "O_RDWR" => ctx.new_int(2),
        "O_NONBLOCK" => ctx.new_int(4),
        "O_APPEND" => ctx.new_int(8),
        "O_CREAT" => ctx.new_int(512)
    })
}
