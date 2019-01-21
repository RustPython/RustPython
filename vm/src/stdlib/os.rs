use std::fs::OpenOptions;
use std::os::unix::io::IntoRawFd;

use num_bigint::{ToBigInt};
use num_traits::cast::ToPrimitive;

use super::super::obj::objstr;
use super::super::obj::objint;
use super::super::obj::objtype;

use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol
};

use super::super::vm::VirtualMachine;

pub fn os_open(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(name, Some(vm.ctx.str_type()))],
        optional = [(mode, Some(vm.ctx.int_type()))] 
    );

    let mode = if let Some(m) = mode {
        objint::get_value(m)
    } else {
        0.to_bigint().unwrap()
    };

    let handle = match mode.to_u16().unwrap() { 
        0 => OpenOptions::new().read(true).open(objstr::get_value(&name)),
        1 => OpenOptions::new().write(true).open(objstr::get_value(&name)),
        512 => OpenOptions::new().write(true).create(true).open(objstr::get_value(&name)),
        _ => OpenOptions::new().read(true).open(objstr::get_value(&name))
    };

    //raw_fd is supported on UNIX only. This will need to be extended
    //to support windows - i.e. raw file_handles 
    if let Ok(f) = handle {
        Ok(vm.ctx.new_int(f.into_raw_fd().to_bigint().unwrap()))
    } else {
        Err(vm.new_value_error("Bad file descriptor".to_string()))
    }
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"io".to_string(), ctx.new_scope(None));
    ctx.set_attr(&py_mod, "open", ctx.new_rustfunc(os_open));
    ctx.set_attr(&py_mod, "O_RDONLY", ctx.new_int(0.to_bigint().unwrap()));
    ctx.set_attr(&py_mod, "O_WRONLY", ctx.new_int(1.to_bigint().unwrap()));
    ctx.set_attr(&py_mod, "O_RDWR", ctx.new_int(2.to_bigint().unwrap()));
    ctx.set_attr(&py_mod, "O_NONBLOCK", ctx.new_int(3.to_bigint().unwrap()));
    ctx.set_attr(&py_mod, "O_CREAT", ctx.new_int(512.to_bigint().unwrap()));
    py_mod
}