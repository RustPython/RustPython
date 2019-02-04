use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::os::unix::io::IntoRawFd;

use num_bigint::ToBigInt;
use num_traits::cast::ToPrimitive;

use super::super::obj::objint;
use super::super::obj::objstr;
use super::super::obj::objtype;

use super::super::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};

use super::super::vm::VirtualMachine;

pub fn os_open(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
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

    //raw_fd is supported on UNIX only. This will need to be extended
    //to support windows - i.e. raw file_handles
    Ok(vm.ctx.new_int(
        handle
            .into_raw_fd()
            .to_bigint()
            .expect("Invalid file descriptor"),
    ))
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"io".to_string(), ctx.new_scope(None));
    ctx.set_attr(&py_mod, "open", ctx.new_rustfunc(os_open));
    ctx.set_attr(&py_mod, "O_RDONLY", ctx.new_int(0.to_bigint().unwrap()));
    ctx.set_attr(&py_mod, "O_WRONLY", ctx.new_int(1.to_bigint().unwrap()));
    ctx.set_attr(&py_mod, "O_RDWR", ctx.new_int(2.to_bigint().unwrap()));
    ctx.set_attr(&py_mod, "O_NONBLOCK", ctx.new_int(4.to_bigint().unwrap()));
    ctx.set_attr(&py_mod, "O_APPEND", ctx.new_int(8.to_bigint().unwrap()));
    ctx.set_attr(&py_mod, "O_CREAT", ctx.new_int(512.to_bigint().unwrap()));
    py_mod
}
