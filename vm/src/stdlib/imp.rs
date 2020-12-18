use crate::builtins::bytes::PyBytesRef;
use crate::builtins::code::PyCode;
use crate::builtins::module::PyModuleRef;
use crate::builtins::pystr::{self, PyStr, PyStrRef};
use crate::import;
use crate::pyobject::{BorrowValue, ItemProtocol, PyObjectRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[cfg(feature = "threading")]
mod lock {
    use crate::pyobject::PyResult;
    use crate::stdlib::thread::RawRMutex;
    use crate::vm::VirtualMachine;

    pub(super) static IMP_LOCK: RawRMutex = RawRMutex::INIT;

    pub(super) fn _imp_acquire_lock(_vm: &VirtualMachine) {
        IMP_LOCK.lock()
    }

    pub(super) fn _imp_release_lock(vm: &VirtualMachine) -> PyResult<()> {
        if !IMP_LOCK.is_locked() {
            Err(vm.new_runtime_error("Global import lock not held".to_owned()))
        } else {
            unsafe { IMP_LOCK.unlock() };
            Ok(())
        }
    }

    pub(super) fn _imp_lock_held(_vm: &VirtualMachine) -> bool {
        IMP_LOCK.is_locked()
    }
}

#[cfg(not(feature = "threading"))]
mod lock {
    use crate::vm::VirtualMachine;
    pub(super) fn _imp_acquire_lock(_vm: &VirtualMachine) {}
    pub(super) fn _imp_release_lock(_vm: &VirtualMachine) {}
    pub(super) fn _imp_lock_held(_vm: &VirtualMachine) -> bool {
        false
    }
}

use lock::{_imp_acquire_lock, _imp_lock_held, _imp_release_lock};

fn _imp_extension_suffixes(vm: &VirtualMachine) -> PyResult {
    Ok(vm.ctx.new_list(vec![]))
}

fn _imp_is_builtin(name: PyStrRef, vm: &VirtualMachine) -> bool {
    vm.state.stdlib_inits.contains_key(name.borrow_value())
}

fn _imp_is_frozen(name: PyStrRef, vm: &VirtualMachine) -> bool {
    vm.state.frozen.contains_key(name.borrow_value())
}

fn _imp_create_builtin(spec: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules").unwrap();
    let spec = vm.get_attribute(spec, "name")?;
    let name = pystr::borrow_value(&spec);

    if let Ok(module) = sys_modules.get_item(name, vm) {
        Ok(module)
    } else if let Some(make_module_func) = vm.state.stdlib_inits.get(name) {
        Ok(make_module_func(vm))
    } else {
        Ok(vm.ctx.none())
    }
}

fn _imp_exec_builtin(_mod: PyModuleRef) -> i32 {
    // TOOD: Should we do something here?
    0
}

fn _imp_get_frozen_object(name: PyStrRef, vm: &VirtualMachine) -> PyResult<PyCode> {
    vm.state
        .frozen
        .get(name.borrow_value())
        .map(|frozen| {
            let mut frozen = frozen.code.clone();
            frozen.source_path = PyStr::from(format!("frozen {}", name)).into_ref(vm);
            PyCode::new(frozen)
        })
        .ok_or_else(|| vm.new_import_error(format!("No such frozen object named {}", name), name))
}

fn _imp_init_frozen(name: PyStrRef, vm: &VirtualMachine) -> PyResult {
    import::import_frozen(vm, name.borrow_value())
}

fn _imp_is_frozen_package(name: PyStrRef, vm: &VirtualMachine) -> PyResult<bool> {
    vm.state
        .frozen
        .get(name.borrow_value())
        .map(|frozen| frozen.package)
        .ok_or_else(|| vm.new_import_error(format!("No such frozen object named {}", name), name))
}

fn _imp_fix_co_filename(_code: PyObjectRef, _path: PyStrRef) {
    // TODO:
}

fn _imp_source_hash(_key: u64, _source: PyBytesRef, vm: &VirtualMachine) -> PyResult {
    // TODO:
    Ok(vm.ctx.none())
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "_imp", {
        "extension_suffixes" => named_function!(ctx, _imp, extension_suffixes),
        "acquire_lock" => named_function!(ctx, _imp, acquire_lock),
        "release_lock" => named_function!(ctx, _imp, release_lock),
        "lock_held" => named_function!(ctx, _imp, lock_held),
        "is_builtin" => named_function!(ctx, _imp, is_builtin),
        "is_frozen" => named_function!(ctx, _imp, is_frozen),
        "create_builtin" => named_function!(ctx, _imp, create_builtin),
        "exec_builtin" => named_function!(ctx, _imp, exec_builtin),
        "get_frozen_object" => named_function!(ctx, _imp, get_frozen_object),
        "init_frozen" => named_function!(ctx, _imp, init_frozen),
        "is_frozen_package" => named_function!(ctx, _imp, is_frozen_package),
        "_fix_co_filename" => named_function!(ctx, _imp, fix_co_filename),
        "source_hash" => named_function!(ctx, _imp, source_hash),
    })
}
