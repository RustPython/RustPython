use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

fn imp_extension_suffixes(vm: &VirtualMachine) -> PyResult {
    Ok(vm.ctx.new_list(vec![]))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    let module = py_module!(vm, "_imp", {
        "extension_suffixes" => ctx.new_rustfunc(imp_extension_suffixes)
    });

    module
}
