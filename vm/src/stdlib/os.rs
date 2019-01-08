use std::fs::File;
use std::os::unix::io::IntoRawFd;

use num_bigint::{BigInt, ToBigInt};

use super::super::obj::objstr;
use super::super::obj::objint;
use super::super::obj::objtype;

use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol, AttributeProtocol
};


use super::super::vm::VirtualMachine;


fn os_open(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(name, Some(vm.ctx.str_type()))]
    );
    match File::open(objstr::get_value(&name)) {
        Ok(v) => Ok(vm.ctx.new_int(v.into_raw_fd().to_bigint().unwrap())),
        Err(v) => Err(vm.new_type_error("Error opening file".to_string())),
    }
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"io".to_string(), ctx.new_scope(None));
    ctx.set_attr(&py_mod, "open", ctx.new_rustfunc(os_open));
    py_mod
}