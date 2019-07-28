use crate::import;
use crate::obj::objcode::PyCode;
use crate::obj::objmodule::PyModuleRef;
use crate::obj::objstr;
use crate::obj::objstr::PyStringRef;
use crate::pyobject::{ItemProtocol, PyObjectRef, PyResult};
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

fn imp_is_frozen(name: PyStringRef, vm: &VirtualMachine) -> bool {
    vm.frozen.borrow().contains_key(name.as_str())
}

fn imp_create_builtin(spec: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules").unwrap();

    let name = &objstr::get_value(&vm.get_attribute(spec.clone(), "name")?);

    if let Ok(module) = sys_modules.get_item(name, vm) {
        Ok(module)
    } else if let Some(make_module_func) = vm.stdlib_inits.borrow().get(name) {
        Ok(make_module_func(vm))
    } else {
        Ok(vm.get_none())
    }
}

fn imp_exec_builtin(_mod: PyModuleRef, _vm: &VirtualMachine) -> i32 {
    // TOOD: Should we do something here?
    0
}

fn imp_get_frozen_object(name: PyStringRef, vm: &VirtualMachine) -> PyResult<PyCode> {
    vm.frozen
        .borrow()
        .get(name.as_str())
        .map(|frozen| {
            let mut frozen = frozen.code.clone();
            frozen.source_path = format!("frozen {}", name.as_str());
            PyCode::new(frozen)
        })
        .ok_or_else(|| {
            vm.new_import_error(format!("No such frozen object named {}", name.as_str()))
        })
}

fn imp_init_frozen(name: PyStringRef, vm: &VirtualMachine) -> PyResult {
    import::import_frozen(vm, name.as_str())
}

fn imp_is_frozen_package(name: PyStringRef, vm: &VirtualMachine) -> PyResult<bool> {
    vm.frozen
        .borrow()
        .get(name.as_str())
        .map(|frozen| frozen.package)
        .ok_or_else(|| {
            vm.new_import_error(format!("No such frozen object named {}", name.as_str()))
        })
}

fn imp_fix_co_filename(_code: PyObjectRef, _path: PyStringRef, _vm: &VirtualMachine) {
    // TODO:
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    let module = py_module!(vm, "_imp", {
        "extension_suffixes" => ctx.new_rustfunc(imp_extension_suffixes),
        "acquire_lock" => ctx.new_rustfunc(imp_acquire_lock),
        "release_lock" => ctx.new_rustfunc(imp_release_lock),
        "lock_held" => ctx.new_rustfunc(imp_lock_held),
        "is_builtin" => ctx.new_rustfunc(imp_is_builtin),
        "is_frozen" => ctx.new_rustfunc(imp_is_frozen),
        "create_builtin" => ctx.new_rustfunc(imp_create_builtin),
        "exec_builtin" => ctx.new_rustfunc(imp_exec_builtin),
        "get_frozen_object" => ctx.new_rustfunc(imp_get_frozen_object),
        "init_frozen" => ctx.new_rustfunc(imp_init_frozen),
        "is_frozen_package" => ctx.new_rustfunc(imp_is_frozen_package),
        "_fix_co_filename" => ctx.new_rustfunc(imp_fix_co_filename),
    });

    module
}
