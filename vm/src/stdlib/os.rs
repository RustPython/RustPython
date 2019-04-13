use std::fs::File;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Read, Write};
use std::{env, fs};

use num_traits::cast::ToPrimitive;

use crate::function::PyFuncArgs;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objdict::PyDictRef;
use crate::obj::objint;
use crate::obj::objint::PyIntRef;
use crate::obj::objstr;
use crate::obj::objstr::PyStringRef;
use crate::pyobject::{ItemProtocol, PyObjectRef, PyResult, TypeProtocol};
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let os_name = if cfg!(windows) {
        "nt".to_string()
    } else {
        "posix".to_string()
    };

    let environ = _os_environ(vm);

    py_module!(vm, "_os", {
        "open" => ctx.new_rustfunc(os_open),
        "close" => ctx.new_rustfunc(os_close),
        "error" => ctx.new_rustfunc(os_error),
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
        "O_RDONLY" => ctx.new_int(0),
        "O_WRONLY" => ctx.new_int(1),
        "O_RDWR" => ctx.new_int(2),
        "O_NONBLOCK" => ctx.new_int(4),
        "O_APPEND" => ctx.new_int(8),
        "O_CREAT" => ctx.new_int(512)
    })
}
