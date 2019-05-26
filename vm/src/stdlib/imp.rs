use crate::obj::objstr::PyStringRef;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

fn imp_extension_suffixes(vm: &VirtualMachine) -> PyResult {
    Ok(vm.ctx.new_list(vec![]))
}

fn imp_acquire_lock(_vm: &VirtualMachine) -> PyResult<()> {
    // TODO
    Ok(())
}

fn imp_release_lock(_vm: &VirtualMachine) -> PyResult<()> {
    // TODO
    Ok(())
}

fn imp_lock_held(_vm: &VirtualMachine) -> PyResult<()> {
    // TODO
    Ok(())
}

fn imp_is_builtin(name: PyStringRef, vm: &VirtualMachine) -> bool {
    vm.stdlib_inits.borrow().contains_key(name.as_str())
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    let module = py_module!(vm, "_imp", {
        "extension_suffixes" => ctx.new_rustfunc(imp_extension_suffixes),
        "acquire_lock" => ctx.new_rustfunc(imp_acquire_lock),
        "release_lock" => ctx.new_rustfunc(imp_release_lock),
        "lock_held" => ctx.new_rustfunc(imp_lock_held),
        "is_builtin" => ctx.new_rustfunc(imp_is_builtin),
    });

    module
}
