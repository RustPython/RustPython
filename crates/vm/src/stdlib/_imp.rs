use crate::builtins::{PyCode, PyStrInterned};
use crate::frozen::FrozenModule;
use crate::{Py, VirtualMachine, builtins::PyBaseExceptionRef};
use core::borrow::Borrow;

pub(crate) use _imp::module_def;

pub(super) use crate::vm::resolve_frozen_alias;

#[cfg(feature = "threading")]
#[pymodule(sub, name = "_imp")]
mod lock {
    use crate::{PyResult, VirtualMachine, stdlib::_thread::RawRMutex};
    use core::cell::Cell;

    static IMP_LOCK: RawRMutex = RawRMutex::INIT;

    thread_local! {
        static IMP_LOCK_DEPTH: Cell<usize> = const { Cell::new(0) };
    }

    fn bump_depth() {
        IMP_LOCK_DEPTH.with(|c| c.set(c.get() + 1));
    }

    fn drop_depth() {
        IMP_LOCK_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    }

    #[pyfunction]
    fn acquire_lock(vm: &VirtualMachine) {
        // Detach while blocking on IMP_LOCK. The import lock is held across
        // bytecode by the importlib bootstrap, so its holder can be parked at a
        // safepoint mid-hold. Blocking here while attached would keep this
        // thread from honoring a stop-the-world request, so a requester could
        // wait for this thread while this thread waits for the parked holder.
        // Detaching makes the wait park-friendly.
        vm.allow_threads(acquire_lock_for_fork);
    }

    #[pyfunction]
    fn release_lock(vm: &VirtualMachine) -> PyResult<()> {
        if !IMP_LOCK.is_locked() || !IMP_LOCK.is_owned_by_current_thread() {
            Err(vm.new_runtime_error("Global import lock not held"))
        } else {
            unsafe { IMP_LOCK.unlock() };
            drop_depth();
            Ok(())
        }
    }

    #[pyfunction]
    fn lock_held(_vm: &VirtualMachine) -> bool {
        IMP_LOCK.is_locked()
    }

    pub(super) fn acquire_lock_for_fork() {
        IMP_LOCK.lock();
        bump_depth();
    }

    #[cfg(all(unix, feature = "host_env"))]
    pub(super) fn release_lock_after_fork_parent() {
        if IMP_LOCK.is_locked() && IMP_LOCK.is_owned_by_current_thread() {
            unsafe { IMP_LOCK.unlock() };
            drop_depth();
        }
    }

    /// Reset the import lock after `fork()`.
    ///
    /// Zero the lock so waiter queues from dead threads are discarded, then
    /// re-acquire it as many times as this thread held it. `unlock()` on the
    /// inherited lock would unpark those waiters.
    ///
    /// # Safety
    ///
    /// Must only be called from single-threaded child after fork().
    #[cfg(all(unix, feature = "host_env"))]
    pub(crate) unsafe fn reinit_after_fork() {
        let depth = IMP_LOCK_DEPTH.with(Cell::get);
        unsafe { rustpython_common::lock::zero_reinit_after_fork(&IMP_LOCK) };
        for _ in 0..depth {
            IMP_LOCK.lock();
        }
    }

    /// Restore the child's import lock, then drop the extra hold taken
    /// before `fork()`. Nested `imp.acquire_lock()` holds stay so the
    /// child's `release_lock()` can unwind them.
    #[cfg(all(unix, feature = "host_env"))]
    pub(super) unsafe fn after_fork_child_reinit_and_release() {
        unsafe { reinit_after_fork() };
        if IMP_LOCK.is_locked() && IMP_LOCK.is_owned_by_current_thread() {
            unsafe { IMP_LOCK.unlock() };
            drop_depth();
        }
    }
}

/// Re-export for fork safety code in posix.rs
///
/// Runs pre-fork on a normal attached VM thread. Detach while blocking so the
/// wait honors a concurrent stop-the-world request instead of pinning this
/// thread attached on IMP_LOCK; re-attach completes before `stop_the_world`, so
/// the fork requester protocol is unaffected.
#[cfg(all(unix, feature = "threading", feature = "host_env"))]
pub(crate) fn acquire_imp_lock_for_fork(vm: &VirtualMachine) {
    vm.allow_threads(lock::acquire_lock_for_fork);
}

#[cfg(all(unix, feature = "threading", feature = "host_env"))]
pub(crate) fn release_imp_lock_after_fork_parent() {
    lock::release_lock_after_fork_parent();
}

#[cfg(all(unix, feature = "threading", feature = "host_env"))]
pub(crate) unsafe fn reinit_imp_lock_after_fork() {
    unsafe { lock::reinit_after_fork() }
}

#[cfg(all(unix, feature = "threading", feature = "host_env"))]
pub(crate) unsafe fn after_fork_child_imp_lock_release() {
    unsafe { lock::after_fork_child_reinit_and_release() }
}

#[cfg(not(feature = "threading"))]
#[pymodule(sub, name = "_imp")]
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
        let msg = match self {
            Self::BadName | Self::NotFound => format!("No such frozen object named {mod_name}"),
            Self::Disabled => format!(
                "Frozen modules are disabled and the frozen object named {mod_name} is not essential"
            ),
            Self::Excluded => format!("Excluded frozen object named {mod_name}"),
            Self::Invalid => format!("Frozen object named {mod_name} is invalid"),
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
        AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyRefExact, PyResult, VirtualMachine,
        builtins::{
            ModuleCreate, PyBytesRef, PyCode, PyDict, PyMemoryView, PyModule, PyStrRef,
            PyUtf8StrRef,
        },
        import, version,
    };
    use core::ffi::c_void;
    use core::ptr::NonNull;
    #[cfg(all(feature = "host_env", windows))]
    use rustpython_host_env::ctypes::open_library;
    #[cfg(all(feature = "host_env", any(unix, windows)))]
    use rustpython_host_env::ctypes::{
        dlopen_mode, insert_raw_library_handle, lookup_function_symbol_addr,
        open_library_with_mode_raw,
    };

    use super::FrozenError;

    // Private exact-dict relocation primitive for future import-cache ordering.
    #[pyfunction]
    fn _dict_move_to_end(
        modules: PyRefExact<PyDict>,
        key: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        modules.move_to_end(key, vm)
    }

    #[pyattr]
    fn check_hash_based_pycs(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx
            .new_str(vm.state.config.settings.check_hash_pycs_mode.to_string())
    }

    #[pyattr(name = "pyc_magic_number_token")]
    use version::PYC_MAGIC_NUMBER_TOKEN;

    #[pyfunction]
    fn extension_suffixes(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        [
            #[cfg(target_os = "macos")]
            ".abi3t-darwin.so",
            #[cfg(unix)]
            ".abi3t.so",
            #[cfg(unix)]
            ".so",
            #[cfg(windows)]
            ".pyd",
        ]
        .into_iter()
        .map(|suffix| vm.ctx.new_str(suffix).into())
        .collect()
    }

    // CPython removes the name from its pending lazy-module registry here.
    // RustPython currently performs only eager imports, so that registry is empty.
    #[pyfunction]
    fn _set_lazy_attributes(
        _modobj: PyObjectRef,
        name: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Even an empty set checks the hash of a str subclass.
        name.as_object().hash(vm)?;
        Ok(())
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
            let module = match def.slots.create {
                Some(ModuleCreate::Rust(create)) => create(vm, &spec, def)?,
                Some(ModuleCreate::C(_)) => {
                    return Err(vm.new_system_error("C module create slot is not supported here"));
                }
                None => PyModule::from_def(def).into_ref(&vm.ctx),
            };

            // Initialize module dict and methods
            // Corresponds to PyModule_FromDefAndSpec: md_def, _add_methods_to_object, PyModule_SetDocString
            PyModule::__init_dict_from_def(vm, &module);
            module.__init_methods(vm)?;

            // Add to sys.modules BEFORE exec (critical for circular import handling)
            sys_modules.set_item(name.as_pystr(), module.clone().into(), vm)?;

            // Phase 2: Call exec slot (can safely import other modules now)
            def.exec_module(vm, &module)?;

            return Ok(module.into());
        }

        Ok(vm.ctx.none())
    }

    #[derive(FromArgs)]
    struct CreateDynamicArgs {
        #[pyarg(positional)]
        spec: PyObjectRef,
        #[pyarg(positional, optional)]
        _file: crate::function::OptionalArg<PyObjectRef>,
    }

    #[cfg(all(feature = "host_env", any(unix, windows)))]
    #[pyfunction]
    fn create_dynamic(args: CreateDynamicArgs, vm: &VirtualMachine) -> PyResult {
        let name_obj = args.spec.get_attr("name", vm)?;
        let name: PyUtf8StrRef = name_obj.try_into_value(vm)?;
        if name.as_str().contains('\0') {
            return Err(vm.new_value_error("embedded null character".to_owned()));
        }

        let origin_obj = args.spec.get_attr("origin", vm)?;
        if vm.is_none(&origin_obj) {
            return Err(vm.new_value_error("origin must be set".to_owned()));
        }
        let origin: PyUtf8StrRef = origin_obj.try_into_value(vm)?;
        if origin.as_str().contains('\0') {
            return Err(vm.new_value_error("embedded null character".to_owned()));
        }

        let sys_modules = vm.sys_module.get_attr("modules", vm)?;
        if let Ok(module) = sys_modules.get_item(&*name, vm) {
            return Ok(module);
        }

        let origin_str = origin.as_str();
        let short_name = name.as_str().rsplit('.').next().unwrap_or(name.as_str());
        let export_func_name = format!("PyModExport_{short_name}");
        let export_func_name_c = format!("{export_func_name}\0");

        let handle = cfg_select! {
            unix => {{
                let mode = dlopen_mode(None);
                open_library_with_mode_raw(origin_str, mode).map(insert_raw_library_handle)
            }},
            windows => {
                open_library(origin_str).map_err(|err| err.to_string())
            },
        }
        .map_err(|err| {
            vm.new_import_error(
                format!("cannot load dynamic module '{origin_str}': {err}"),
                name.clone().into_wtf8(),
            )
        })?;

        let export_fn_addr = lookup_function_symbol_addr(handle, export_func_name_c.as_bytes())
            .or_else(|_err| {
                cfg_select! {
                    target_os = "macos" => {{
                        let mangled = format!("_{export_func_name}");
                        let mangled_c = format!("{mangled}\0");
                        lookup_function_symbol_addr(handle, mangled_c.as_bytes())
                    }}
                    _ => Err(_err),
                }
            })
            .map_err(|_| {
                vm.new_import_error(
                    format!(
                        "dynamic module does not define module export function (PyModExport_{short_name})",
                    ),
                    name.clone().into_wtf8(),
                )
            })?;

        unsafe extern "C" {
            #[allow(improper_ctypes)]
            /// This function is defined in the rustpython-capi crate.
            fn PyModule_FromSlotsAndSpec(slots: *mut c_void, spec: *mut PyObject) -> *mut PyObject;
        }

        let export_fn: unsafe extern "C" fn() -> *mut c_void =
            unsafe { core::mem::transmute(export_fn_addr) };
        let module_ptr = unsafe {
            PyModule_FromSlotsAndSpec(export_fn(), args.spec.as_object().as_raw().cast_mut())
        };
        NonNull::new(module_ptr).map_or_else(
            || Err(vm.take_raised_exception().unwrap()),
            |ptr| unsafe { Ok(PyObjectRef::from_raw(ptr)) },
        )
    }

    #[pyfunction]
    fn exec_dynamic(module: PyRef<PyModule>, vm: &VirtualMachine) -> PyResult<()> {
        let def = module
            .def
            .as_deref()
            .ok_or_else(|| vm.new_system_error("Empty module"))?;
        def.exec_module(vm, &module)
    }

    #[pyfunction]
    fn exec_builtin(_mod: PyRef<PyModule>) -> i32 {
        // For multi-phase init modules, exec is already called in create_builtin
        0
    }

    #[derive(FromArgs)]
    struct FrozenObjectArgs {
        #[pyarg(positional)]
        name: PyUtf8StrRef,
        #[pyarg(positional, optional)]
        data: Option<PyObjectRef>,
    }

    #[pyfunction]
    fn get_frozen_object(args: FrozenObjectArgs, vm: &VirtualMachine) -> PyResult<PyRef<PyCode>> {
        let FrozenObjectArgs { name, data } = args;
        if let Some(data) = data
            && !vm.is_none(&data)
        {
            let invalid_err = || {
                vm.new_import_error(
                    format!("Frozen object named '{}' is invalid", name.as_str()),
                    name.clone().into_wtf8(),
                )
            };
            // A non-buffer is a TypeError, not invalid frozen data. The request
            // is the one marshal.loads() makes, so that what passes here is
            // exactly what it accepts.
            crate::protocol::PyBuffer::from_object(
                vm,
                &data,
                crate::protocol::BufferFlags::SIMPLE,
            )?;
            // The data is a marshalled code object: a whole marshal value, which
            // deserialize_code() does not read — it takes the code body alone,
            // without the type byte the writer puts in front of it.
            let loads = vm.import("marshal", 0)?.get_attr("loads", vm)?;
            let code = loads.call((data,), vm).map_err(|_| invalid_err())?;
            return code.downcast::<PyCode>().map_err(|_| invalid_err());
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
    fn _override_frozen_modules_for_tests(r#override: isize, vm: &VirtualMachine) {
        vm.state.override_frozen_modules.store(r#override);
    }

    #[pyfunction]
    fn _fix_co_filename(code: PyRef<PyCode>, path: PyStrRef, vm: &VirtualMachine) {
        let old_name = code.source_path();
        let new_name = vm.ctx.intern_str(path.as_wtf8());
        super::update_code_filenames(&code, old_name, new_name);
    }

    #[pyfunction]
    fn _frozen_module_names(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vm.state
            .frozen
            .keys()
            .map(|&name| vm.ctx.new_utf8_str(name).into())
            .collect()
    }

    #[derive(FromArgs)]
    struct FindFrozenArgs {
        #[pyarg(positional)]
        name: PyUtf8StrRef,
        #[pyarg(named, default)]
        withdata: bool,
    }

    #[allow(clippy::type_complexity)]
    #[pyfunction]
    fn find_frozen(
        args: FindFrozenArgs,
        vm: &VirtualMachine,
    ) -> PyResult<Option<(Option<PyRef<PyMemoryView>>, bool, Option<PyStrRef>)>> {
        let FindFrozenArgs { name, withdata } = args;

        let name_str = name.as_str();
        let info = match super::find_frozen(name_str, vm) {
            Ok(info) => info,
            Err(FrozenError::NotFound | FrozenError::Disabled | FrozenError::BadName) => {
                return Ok(None);
            }
            Err(e) => return Err(e.to_pyexception(name_str, vm)),
        };

        // The data is what get_frozen_object() takes back, i.e. marshalled code.
        // Frozen modules are stored in their own encoding, so it has to be
        // re-serialized rather than handed out as a view of the stored bytes.
        let data = if withdata {
            let code = PyCode::new_ref_from_frozen(vm, info.code);
            let dumps = vm.import("marshal", 0)?.get_attr("dumps", vm)?;
            let bytes = dumps.call((code,), vm)?;
            Some(PyMemoryView::from_object(&bytes, vm)?.into_ref(&vm.ctx))
        } else {
            None
        };

        // When origname is empty (e.g. __hello_only__), return None.
        // Otherwise return the resolved alias name.
        let origname_str = super::resolve_frozen_alias(name_str);
        let origname = if origname_str.is_empty() {
            None
        } else {
            Some(vm.ctx.new_utf8_str(origname_str).into())
        };
        Ok(Some((data, info.package, origname)))
    }

    #[derive(FromArgs)]
    struct SourceHashArgs {
        #[pyarg(any)]
        key: u64,
        #[pyarg(any)]
        source: PyBytesRef,
    }

    #[pyfunction]
    fn source_hash(SourceHashArgs { key, source }: SourceHashArgs) -> Vec<u8> {
        let hash: u64 = crate::common::hash::keyed_hash(key, source.as_bytes());
        hash.to_le_bytes().to_vec()
    }
}

fn update_code_filenames(
    code: &Py<PyCode>,
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
