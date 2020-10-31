use crate::pyobject::PyClassImpl;
use crate::pyobject::PyObjectRef;
use crate::VirtualMachine;

mod dll;
mod function;

use crate::stdlib::ctypes::dll::*;
use crate::stdlib::ctypes::function::*;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "_ctypes", {
        "dlopen" => ctx.new_function(dlopen),
        "dlsym" => ctx.new_function(dlsym),
        // "CFuncPtr" => ctx.new_class("CFuncPtr", &ctx.types.object_type, Default::default())
        "CFuncPtr" => PyCFuncPtr::make_class(ctx),
    })
}
