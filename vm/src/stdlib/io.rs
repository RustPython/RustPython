/*
 * I/O core tools.
 */

// use super::super::obj::{objstr, objtype};
use super::super::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult};
use super::super::VirtualMachine;

fn string_io_init(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    // arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    // TODO
    Ok(vm.get_none())
}

fn string_io_getvalue(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    // TODO
    Ok(vm.get_none())
}

fn bytes_io_init(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    // TODO
    Ok(vm.get_none())
}

fn bytes_io_getvalue(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    // TODO
    Ok(vm.get_none())
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    py_module!(ctx, io {
        IOBase: (class {}),
        StringIO: (class(IOBase) {
            __init__: (func string_io_init),
            getvalue: (func string_io_getvalue),
        }),
        BytesIO: (class(IOBase) {
            __init__: (func bytes_io_init),
            getvalue: (func bytes_io_getvalue)
        })
    })
}
