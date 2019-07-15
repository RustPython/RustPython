use crate::obj::objstr::PyStringRef;
use crate::pyobject::{PyCallable, PyObjectRef, PyResult};
use crate::VirtualMachine;

fn codecs_lookup_error(name: PyStringRef, vm: &VirtualMachine) -> PyResult {
    Err(vm.new_exception(
        vm.ctx.exceptions.lookup_error.clone(),
        format!("unknown error handler name '{}'", name.as_str()),
    ))
}

fn codecs_register(_search_func: PyCallable, _vm: &VirtualMachine) {
    // TODO
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "_codecs", {
        "lookup_error" => vm.ctx.new_rustfunc(codecs_lookup_error),
        "register" => vm.ctx.new_rustfunc(codecs_register),
    })
}
