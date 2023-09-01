use crate::frozen::FrozenModule;
use crate::{builtins::PyBaseExceptionRef, VirtualMachine};
pub(crate) use _imp::make_module;

#[cfg(feature = "threading")]
#[pymodule(sub)]
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
#[pymodule(sub)]
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

#[allow(dead_code)]
enum FrozenError {
    BadName,  // The given module name wasn't valid.
    NotFound, // It wasn't in PyImport_FrozenModules.
    Disabled, // -X frozen_modules=off (and not essential)
    Excluded, // The PyImport_FrozenModules entry has NULL "code"
    //        (module is present but marked as unimportable, stops search).
    Invalid, // The PyImport_FrozenModules entry is bogus
             //          (eg. does not contain executable code).
}

impl FrozenError {
    fn to_pyexception(&self, mod_name: &str, vm: &VirtualMachine) -> PyBaseExceptionRef {
        use FrozenError::*;
        let msg = match self {
            BadName | NotFound => format!("No such frozen object named {mod_name}"),
            Disabled => format!("Frozen modules are disabled and the frozen object named {mod_name} is not essential"),
            Excluded => format!("Excluded frozen object named {mod_name}"),
            Invalid => format!("Frozen object named {mod_name} is invalid"),
        };
        vm.new_import_error(msg, vm.ctx.new_str(mod_name))
    }
}

// find_frozen in frozen.c
fn find_frozen(name: &str, vm: &VirtualMachine) -> Result<FrozenModule, FrozenError> {
    vm.state
        .frozen
        .get(name)
        .copied()
        .ok_or(FrozenError::NotFound)
}

#[pymodule(with(lock))]
mod _imp {
    use crate::{
        builtins::{PyBytesRef, PyCode, PyMemoryView, PyModule, PyStrRef},
        function::OptionalArg,
        import, PyObjectRef, PyRef, PyResult, VirtualMachine,
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
        let name: PyStrRef = spec.get_attr("name", vm)?.try_into_value(vm)?;

        let module = if let Ok(module) = sys_modules.get_item(&*name, vm) {
            module
        } else if let Some(make_module_func) = vm.state.module_inits.get(name.as_str()) {
            make_module_func(vm).into()
        } else {
            vm.ctx.none()
        };
        Ok(module)
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
        super::find_frozen(name.as_str(), vm)
            .map(|frozen| frozen.package)
            .map_err(|e| e.to_pyexception(name.as_str(), vm))
    }

    #[pyfunction]
    fn _override_frozen_modules_for_tests(value: isize, vm: &VirtualMachine) {
        vm.state.override_frozen_modules.store(value);
    }

    #[pyfunction]
    fn _fix_co_filename(_code: PyObjectRef, _path: PyStrRef) {
        // TODO:
    }

    #[pyfunction]
    fn _frozen_module_names(vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let names = vm
            .state
            .frozen
            .keys()
            .map(|&name| vm.ctx.new_str(name).into())
            .collect();
        Ok(names)
    }

    #[allow(clippy::type_complexity)]
    #[pyfunction]
    fn find_frozen(
        name: PyStrRef,
        withdata: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<(Option<PyRef<PyMemoryView>>, bool, PyStrRef)>> {
        use super::FrozenError::*;

        if withdata.into_option().is_some() {
            // this is keyword-only argument in CPython
            unimplemented!();
        }

        let info = match super::find_frozen(name.as_str(), vm) {
            Ok(info) => info,
            Err(NotFound | Disabled | BadName) => return Ok(None),
            Err(e) => return Err(e.to_pyexception(name.as_str(), vm)),
        };

        let origname = name; // FIXME: origname != name
        Ok(Some((None, info.package, origname)))
    }

    #[pyfunction]
    fn source_hash(key: u64, source: PyBytesRef) -> Vec<u8> {
        let hash: u64 = crate::common::hash::keyed_hash(key, source.as_bytes());
        hash.to_le_bytes().to_vec()
    }
}
