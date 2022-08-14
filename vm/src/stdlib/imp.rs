use crate::{PyObjectRef, VirtualMachine};

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = _imp::make_module(vm);
    lock::extend_module(vm, &module);
    module
}

#[cfg(feature = "threading")]
#[pymodule]
mod lock {
    use crate::{stdlib::thread::RawRMutex, PyResult, VirtualMachine};

    static IMP_LOCK: RawRMutex = RawRMutex::INIT;

    #[pyfunction]
    fn acquire_lock(_vm: &VirtualMachine) {
        IMP_LOCK.lock()
    }

    #[pyfunction]
    fn release_lock(vm: &VirtualMachine) -> PyResult<()> {
        if !IMP_LOCK.is_locked() {
            Err(vm.new_runtime_error("Global import lock not held".to_owned()))
        } else {
            unsafe { IMP_LOCK.unlock() };
            Ok(())
        }
    }

    #[pyfunction]
    fn lock_held(_vm: &VirtualMachine) -> bool {
        IMP_LOCK.is_locked()
    }
}

#[cfg(not(feature = "threading"))]
#[pymodule]
mod lock {
    use crate::vm::VirtualMachine;
    #[pyfunction]
    pub(super) fn acquire_lock(_vm: &VirtualMachine) {}
    #[pyfunction]
    pub(super) fn release_lock(_vm: &VirtualMachine) {}
    #[pyfunction]
    pub(super) fn lock_held(_vm: &VirtualMachine) -> bool {
        false
    }
}

#[pymodule]
mod _imp {
    use crate::{
        builtins::{PyBytesRef, PyCode, PyModule, PyStrRef},
        import, PyObjectRef, PyRef, PyResult, TryFromObject, VirtualMachine,
    };

    #[pyattr]
    fn check_hash_based_pycs(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx
            .new_str(vm.state.settings.check_hash_based_pycs.clone())
    }

    #[pyfunction]
    fn extension_suffixes() -> PyResult<Vec<PyObjectRef>> {
        Ok(Vec::new())
    }

    #[pyfunction]
    fn is_builtin(name: PyStrRef, vm: &VirtualMachine) -> bool {
        vm.state.module_inits.contains_key(name.as_str())
    }

    #[pyfunction]
    fn is_frozen(name: PyStrRef, vm: &VirtualMachine) -> bool {
        vm.state.frozen.contains_key(name.as_str())
    }

    #[pyfunction]
    fn create_builtin(spec: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let sys_modules = vm.sys_module.get_attr("modules", vm).unwrap();
        let name = spec.get_attr("name", vm)?;
        let name = PyStrRef::try_from_object(vm, name)?;

        if let Ok(module) = sys_modules.get_item(&*name, vm) {
            Ok(module)
        } else if let Some(make_module_func) = vm.state.module_inits.get(name.as_str()) {
            Ok(make_module_func(vm))
        } else {
            Ok(vm.ctx.none())
        }
    }

    #[pyfunction]
    fn exec_builtin(_mod: PyRef<PyModule>) -> i32 {
        // TODO: Should we do something here?
        0
    }

    #[pyfunction]
    fn get_frozen_object(name: PyStrRef, vm: &VirtualMachine) -> PyResult<PyRef<PyCode>> {
        import::make_frozen(vm, name.as_str())
    }

    #[pyfunction]
    fn init_frozen(name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        import::import_frozen(vm, name.as_str())
    }

    #[pyfunction]
    fn is_frozen_package(name: PyStrRef, vm: &VirtualMachine) -> PyResult<bool> {
        vm.state
            .frozen
            .get(name.as_str())
            .map(|frozen| frozen.package)
            .ok_or_else(|| {
                vm.new_import_error(format!("No such frozen object named {}", name), name)
            })
    }

    #[pyfunction]
    fn _fix_co_filename(_code: PyObjectRef, _path: PyStrRef) {
        // TODO:
    }

    #[pyfunction]
    fn source_hash(key: u64, source: PyBytesRef) -> Vec<u8> {
        let hash: u64 = crate::common::hash::keyed_hash(key, source.as_bytes());
        hash.to_le_bytes().to_vec()
    }
}
