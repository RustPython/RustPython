use crate::pyobject::PyClassImpl;
use crate::pyobject::PyObjectRef;
use crate::VirtualMachine;

mod basics;
mod dll;
mod function;
mod primitive;
mod shared_lib;

use crate::stdlib::ctypes::dll::*;
use crate::stdlib::ctypes::function::*;
use crate::stdlib::ctypes::primitive::*;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "_ctypes", {
        "dlopen" => ctx.new_function(dlopen),
        "dlsym" => ctx.new_function(dlsym),
        "dlclose" => ctx.new_function(dlclose),

        "CFuncPtr" => PyCFuncPtr::make_class(ctx),
        "_SimpleCData" => PySimpleType::make_class(ctx)
    })
}
