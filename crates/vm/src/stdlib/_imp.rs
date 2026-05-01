use crate::builtins::{PyCode, PyStrInterned};
use crate::frozen::FrozenModule;
use crate::{VirtualMachine, builtins::PyBaseExceptionRef};
use core::borrow::Borrow;

pub(crate) use _imp::module_def;

pub use crate::vm::resolve_frozen_alias;

#[cfg(feature = "threading")]
#[pymodule(sub)]
mod lock {
    use crate::{PyResult, VirtualMachine, stdlib::_thread::RawRMutex};

    static IMP_LOCK: RawRMutex = RawRMutex::INIT;

    #[pyfunction]
    fn acquire_lock(_vm: &VirtualMachine) {
        acquire_lock_for_fork()
    }

    #[pyfunction]
    fn release_lock(vm: &VirtualMachine) -> PyResult<()> {
        if !IMP_LOCK.is_locked() {
            Err(vm.new_runtime_error("Global import lock not held"))
        } else {
            unsafe { IMP_LOCK.unlock() };
            Ok(())
        }
    }

    #[pyfunction]
    fn lock_held(_vm: &VirtualMachine) -> bool {
        IMP_LOCK.is_locked()
    }

    pub(super) fn acquire_lock_for_fork() {
        IMP_LOCK.lock();
    }

    pub(super) fn release_lock_after_fork_parent() {
        if IMP_LOCK.is_locked() && IMP_LOCK.is_owned_by_current_thread() {
            unsafe { IMP_LOCK.unlock() };
        }
    }

    /// Reset import lock after fork() — only if held by a dead thread.
    ///
    /// `IMP_LOCK` is a reentrant mutex. If the *current* (surviving) thread
    /// held it at fork time, the child must be able to release it normally.
    /// Only reset if a now-dead thread was the owner.
    ///
    /// # Safety
    ///
    /// Must only be called from single-threaded child after fork().
    #[cfg(unix)]
    pub(crate) unsafe fn reinit_after_fork() {
        if IMP_LOCK.is_locked() && !IMP_LOCK.is_owned_by_current_thread() {
            // Held by a dead thread — reset to unlocked.
            unsafe { rustpython_common::lock::zero_reinit_after_fork(&IMP_LOCK) };
        }
    }

    /// Match CPython's `_PyImport_ReInitLock()` + `_PyImport_ReleaseLock()`
    /// behavior in the post-fork child:
    /// 1) if ownership metadata is stale (dead owner / changed tid), reset;
    /// 2) if current thread owns the lock, release it.
    #[cfg(unix)]
    pub(super) unsafe fn after_fork_child_reinit_and_release() {
        unsafe { reinit_after_fork() };
        if IMP_LOCK.is_locked() && IMP_LOCK.is_owned_by_current_thread() {
            unsafe { IMP_LOCK.unlock() };
        }
    }
}

/// Re-export for fork safety code in posix.rs
#[cfg(feature = "threading")]
pub(crate) fn acquire_imp_lock_for_fork() {
    lock::acquire_lock_for_fork();
}

#[cfg(feature = "threading")]
pub(crate) fn release_imp_lock_after_fork_parent() {
    lock::release_lock_after_fork_parent();
}

#[cfg(all(unix, feature = "threading"))]
pub(crate) unsafe fn reinit_imp_lock_after_fork() {
    unsafe { lock::reinit_after_fork() }
}

#[cfg(all(unix, feature = "threading"))]
pub(crate) unsafe fn after_fork_child_imp_lock_release() {
    unsafe { lock::after_fork_child_reinit_and_release() }
}

#[cfg(not(feature = "threading"))]
#[pymodule(sub)]
mod lock {
    use crate::vm::VirtualMachine;
    #[pyfunction]
    pub(super) const fn acquire_lock(_vm: &VirtualMachine) {}
    #[pyfunction]
    pub(super) const fn release_lock(_vm: &VirtualMachine) {}
    #[pyfunction]
    pub(super) const fn lock_held(_vm: &VirtualMachine) -> bool {
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
            Disabled => format!(
                "Frozen modules are disabled and the frozen object named {mod_name} is not essential"
            ),
            Excluded => format!("Excluded frozen object named {mod_name}"),
            Invalid => format!("Frozen object named {mod_name} is invalid"),
        };
        vm.new_import_error(msg, vm.ctx.new_utf8_str(mod_name))
    }
}

// look_up_frozen + use_frozen in import.c
fn find_frozen(name: &str, vm: &VirtualMachine) -> Result<FrozenModule, FrozenError> {
    let frozen = vm
        .state
        .frozen
        .get(name)
        .copied()
        .ok_or(FrozenError::NotFound)?;

    // Bootstrap modules are always available regardless of override flag
    if matches!(
        name,
        "_frozen_importlib" | "_frozen_importlib_external" | "zipimport"
    ) {
        return Ok(frozen);
    }

    // use_frozen(): override > 0 → true, override < 0 → false, 0 → default (true)
    // When disabled, non-bootstrap modules are simply not found (same as look_up_frozen)
    let override_val = vm.state.override_frozen_modules.load();
    if override_val < 0 {
        return Err(FrozenError::NotFound);
    }

    Ok(frozen)
}

#[pymodule(with(lock))]
mod _imp {
    use crate::{
        AsObject,
        PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{
            PyBaseException, PyBytesRef, PyCode, PyMemoryView, PyModule, PyStrRef, PyUtf8StrRef,
        },
        convert::TryFromBorrowedObject,
        function::OptionalArg,
        import, version,
    };
    use alloc::ffi::CString;
    use core::ffi::c_int;

    #[pyattr]
    fn check_hash_based_pycs(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx
            .new_str(vm.state.config.settings.check_hash_pycs_mode.to_string())
    }

    #[pyattr(name = "pyc_magic_number_token")]
    use version::PYC_MAGIC_NUMBER_TOKEN;

    #[pyfunction]
    fn extension_suffixes(vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let version = format!("{}{}", crate::version::MAJOR, crate::version::MINOR);
        let rustpython_suffix = format!(".rustpython{version}-{}.so", crate::stdlib::sys::multiarch());
        let cpython_suffix = format!(
            ".cpython-{version}-{}.so",
            crate::stdlib::sys::cpython_ext_platform_tag()
        );
        Ok(vec![
            vm.ctx.new_str(cpython_suffix).into(),
            vm.ctx.new_str(".abi3.so").into(),
            vm.ctx.new_str(rustpython_suffix).into(),
        ])
    }

    #[pyfunction]
    fn is_builtin(name: PyUtf8StrRef, vm: &VirtualMachine) -> bool {
        vm.state.module_defs.contains_key(name.as_str())
    }

    #[pyfunction]
    fn is_frozen(name: PyUtf8StrRef, vm: &VirtualMachine) -> bool {
        super::find_frozen(name.as_str(), vm).is_ok()
    }

    #[pyfunction]
    fn create_builtin(spec: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let sys_modules = vm.sys_module.get_attr("modules", vm).unwrap();
        let name: PyUtf8StrRef = spec.get_attr("name", vm)?.try_into_value(vm)?;

        // Check sys.modules first
        if let Ok(module) = sys_modules.get_item(&*name, vm) {
            return Ok(module);
        }

        let name_str = name.as_str();
        if let Some(&def) = vm.state.module_defs.get(name_str) {
            // Phase 1: Create module (use create slot if provided, else default creation)
            let module = if let Some(create) = def.slots.create {
                // Custom module creation
                create(vm, &spec, def)?
            } else {
                // Default module creation
                PyModule::from_def(def).into_ref(&vm.ctx)
            };

            // Initialize module dict and methods
            // Corresponds to PyModule_FromDefAndSpec: md_def, _add_methods_to_object, PyModule_SetDocString
            PyModule::__init_dict_from_def(vm, &module);
            module.__init_methods(vm)?;

            // Add to sys.modules BEFORE exec (critical for circular import handling)
            sys_modules.set_item(name.as_pystr(), module.clone().into(), vm)?;

            // Phase 2: Call exec slot (can safely import other modules now)
            if let Some(exec) = def.slots.exec {
                exec(vm, &module)?;
            }

            return Ok(module.into());
        }

        Ok(vm.ctx.none())
    }

    #[pyfunction]
    fn exec_builtin(_mod: PyRef<PyModule>) -> i32 {
        // For multi-phase init modules, exec is already called in create_builtin
        0
    }

    #[pyfunction]
    fn create_dynamic(spec: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        type CreateDynamicFn = unsafe extern "C" fn(*mut crate::PyObject) -> *mut crate::PyObject;
        type TakeDynamicErrorFn = unsafe extern "C" fn() -> *mut crate::PyObject;
        let name: PyUtf8StrRef = spec.get_attr("name", vm)?.try_into_value(vm)?;
        let symbol_name = CString::new("RustPython_CreateDynamicExtension").unwrap();
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, symbol_name.as_ptr()) };
        if symbol.is_null() {
            return Err(vm.new_import_error(
                "no external dynamic extension loader registered",
                name.clone().into_wtf8(),
            ));
        }
        let create_dynamic: CreateDynamicFn = unsafe { core::mem::transmute(symbol) };
        let raw_module = unsafe { create_dynamic(spec.as_raw().cast_mut()) };
        if raw_module.is_null() {
            let err_symbol_name = CString::new("RustPython_TakeDynamicExtensionError").unwrap();
            let err_symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, err_symbol_name.as_ptr()) };
            if !err_symbol.is_null() {
                let take_error: TakeDynamicErrorFn = unsafe { core::mem::transmute(err_symbol) };
                let raw_err = unsafe { take_error() };
                if !raw_err.is_null() {
                    let err_obj = unsafe { (&*raw_err).to_owned() };
                    if let Ok(err) =
                        err_obj.downcast::<PyBaseException>()
                    {
                        return Err(err);
                    }
                }
            }
            return Err(vm
                .take_raised_exception()
                .or_else(|| vm.current_exception())
                .unwrap_or_else(|| {
                    vm.new_import_error(
                        format!("native module create failed for {}", name.as_str()),
                        name.clone().into_wtf8(),
                    )
                }));
        }
        Ok(unsafe { (&*raw_module).to_owned() })
    }

    #[pyfunction]
    fn exec_dynamic(mod_: PyRef<PyModule>, vm: &VirtualMachine) -> PyResult<c_int> {
        type ExecDynamicFn = unsafe extern "C" fn(*mut crate::PyObject) -> c_int;
        type TakeDynamicErrorFn = unsafe extern "C" fn() -> *mut crate::PyObject;
        let symbol_name = CString::new("RustPython_ExecDynamicExtension").unwrap();
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, symbol_name.as_ptr()) };
        if symbol.is_null() {
            return Ok(0);
        }
        let exec_dynamic: ExecDynamicFn = unsafe { core::mem::transmute(symbol) };
        let rc = unsafe { exec_dynamic(mod_.as_object().as_raw().cast_mut()) };
        if rc != 0 {
            let err_symbol_name = CString::new("RustPython_TakeDynamicExtensionError").unwrap();
            let err_symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, err_symbol_name.as_ptr()) };
            if !err_symbol.is_null() {
                let take_error: TakeDynamicErrorFn = unsafe { core::mem::transmute(err_symbol) };
                let raw_err = unsafe { take_error() };
                if !raw_err.is_null() {
                    if let Ok(err) =
                        unsafe { (&*raw_err).to_owned() }.downcast::<PyBaseException>()
                    {
                        return Err(err);
                    }
                }
            }
            if let Some(err) = vm.take_raised_exception().or_else(|| vm.current_exception()) {
                return Err(err);
            }
            return Err(vm.new_import_error(
                "native module exec failed",
                vm.ctx.new_utf8_str("<extension>"),
            ));
        }
        Ok(0)
    }

    #[pyfunction]
    fn get_frozen_object(
        name: PyUtf8StrRef,
        data: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyCode>> {
        if let OptionalArg::Present(data) = data
            && !vm.is_none(&data)
        {
            let buf = crate::protocol::PyBuffer::try_from_borrowed_object(vm, &data)?;
            let contiguous = buf.as_contiguous().ok_or_else(|| {
                vm.new_buffer_error("get_frozen_object() requires a contiguous buffer")
            })?;
            let invalid_err = || {
                vm.new_import_error(
                    format!("Frozen object named '{}' is invalid", name.as_str()),
                    name.clone().into_wtf8(),
                )
            };
            let bag = crate::builtins::code::PyVmBag(vm);
            let code =
                rustpython_compiler_core::marshal::deserialize_code(&mut &contiguous[..], bag)
                    .map_err(|_| invalid_err())?;
            return Ok(PyCode::new_ref_with_bag(vm, code));
        }
        import::make_frozen(vm, name.as_str())
    }

    #[pyfunction]
    fn init_frozen(name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult {
        import::import_frozen(vm, name.as_str())
    }

    #[pyfunction]
    fn is_frozen_package(name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<bool> {
        let name_str = name.as_str();
        super::find_frozen(name_str, vm)
            .map(|frozen| frozen.package)
            .map_err(|e| e.to_pyexception(name_str, vm))
    }

    #[pyfunction]
    fn _override_frozen_modules_for_tests(value: isize, vm: &VirtualMachine) {
        vm.state.override_frozen_modules.store(value);
    }

    #[pyfunction]
    fn _fix_co_filename(code: PyRef<PyCode>, path: PyStrRef, vm: &VirtualMachine) {
        let old_name = code.source_path();
        let new_name = vm.ctx.intern_str(path.as_wtf8());
        super::update_code_filenames(&code, old_name, new_name);
    }

    #[pyfunction]
    fn _frozen_module_names(vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let names = vm
            .state
            .frozen
            .keys()
            .map(|&name| vm.ctx.new_utf8_str(name).into())
            .collect();
        Ok(names)
    }

    #[allow(clippy::type_complexity)]
    #[pyfunction]
    fn find_frozen(
        name: PyUtf8StrRef,
        withdata: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<(Option<PyRef<PyMemoryView>>, bool, Option<PyStrRef>)>> {
        use super::FrozenError::*;

        if withdata.into_option().is_some() {
            // this is keyword-only argument in CPython
            unimplemented!();
        }

        let name_str = name.as_str();
        let info = match super::find_frozen(name_str, vm) {
            Ok(info) => info,
            Err(NotFound | Disabled | BadName) => return Ok(None),
            Err(e) => return Err(e.to_pyexception(name_str, vm)),
        };

        // When origname is empty (e.g. __hello_only__), return None.
        // Otherwise return the resolved alias name.
        let origname_str = super::resolve_frozen_alias(name_str);
        let origname = if origname_str.is_empty() {
            None
        } else {
            Some(vm.ctx.new_utf8_str(origname_str).into())
        };
        Ok(Some((None, info.package, origname)))
    }

    #[pyfunction]
    fn source_hash(key: u64, source: PyBytesRef) -> Vec<u8> {
        let hash: u64 = crate::common::hash::keyed_hash(key, source.as_bytes());
        hash.to_le_bytes().to_vec()
    }
}

fn update_code_filenames(
    code: &PyCode,
    old_name: &'static PyStrInterned,
    new_name: &'static PyStrInterned,
) {
    let current = code.source_path();
    if !core::ptr::eq(current, old_name) && current.as_str() != old_name.as_str() {
        return;
    }
    code.set_source_path(new_name);
    for constant in code.code.constants.iter() {
        let obj: &crate::PyObject = constant.borrow();
        if let Some(inner_code) = obj.downcast_ref::<PyCode>() {
            update_code_filenames(inner_code, old_name, new_name);
        }
    }
}
