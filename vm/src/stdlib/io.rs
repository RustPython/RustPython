/*
 * I/O core tools.
 */

// use super::super::obj::{objstr, objtype};
use super::super::pyobject::{
    AttributeProtocol, DictProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult,
};
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
    let py_mod = ctx.new_module(&"io".to_string(), ctx.new_scope(None));

    let io_base = ctx.new_class("IOBase", ctx.object());
    py_mod.set_item("IOBase", io_base.clone());

    let string_io = ctx.new_class("StringIO", io_base.clone());
    string_io.set_attr("__init__", ctx.new_rustfunc(string_io_init));
    string_io.set_attr("getvalue", ctx.new_rustfunc(string_io_getvalue));
    py_mod.set_item("StringIO", string_io);

    let bytes_io = ctx.new_class("BytesIO", io_base.clone());
    bytes_io.set_attr("__init__", ctx.new_rustfunc(bytes_io_init));
    bytes_io.set_attr("getvalue", ctx.new_rustfunc(bytes_io_getvalue));
    py_mod.set_item("BytesIO", bytes_io);

    py_mod
}
