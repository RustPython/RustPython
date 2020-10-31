use crate::pyobject::PyObjectRef;
use crate::VirtualMachine;

mod dll;

use crate::stdlib::ctypes::dll::*;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "_ctypes", {
        "dlopen" => ctx.new_function(dlopen),
        "dlsym" => ctx.new_function(dlsym),
    })
}
