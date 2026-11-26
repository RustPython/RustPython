//! Import of abi3t extension modules through their `PyModExport_<name>` entry point.

use crate::methodobject::PyMethodDef;
use crate::slots::{PySlot, PySlotKind, PySlotModule};
use crate::util::CStrExt;
use core::ffi::{CStr, c_int, c_void};
use rustpython_vm::builtins::{PyCapsule, PyTuple};
use rustpython_vm::{AsObject, PyObject, PyPayload, PyResult, VirtualMachine};

type ExecSlot = unsafe extern "C" fn(*mut PyObject) -> c_int;

/// Key under which the exec slots of a created module wait in its dict until
/// `_imp.exec_dynamic` runs them, once importlib has set the module attributes and added the
/// module to `sys.modules` (PEP 489). Living in the module dict, they go away with the module.
const PENDING_EXEC_KEY: &str = "__rustpython_pending_exec__";
const PENDING_EXEC_NAME: &CStr = c"rustpython.pending_exec";

/// Make `_imp.create_dynamic` able to load extension modules built for the stable ABI.
///
/// importlib reads the extension suffixes while the interpreter is built, so code that
/// builds the interpreter itself must call this before `Interpreter::builder`.
pub fn register_extension_loader() {
    rustpython_vm::stdlib::set_extension_module_loader(
        load_extension_module,
        exec_extension_module,
    );
}

fn load_extension_module(vm: &VirtualMachine, name: &str, export_fn: usize) -> PyResult {
    type ExportFn = unsafe extern "C" fn() -> *const PySlot;
    // Safety: the address comes from a successful lookup of `PyModExport_<name>`.
    let export: ExportFn = unsafe { core::mem::transmute(export_fn as *const ()) };
    let slots = unsafe { export() };
    if slots.is_null() {
        return Err(vm.take_raised_exception().unwrap_or_else(|| {
            vm.new_system_error(format!(
                "PyModExport_{name} returned NULL without an exception"
            ))
        }));
    }

    let mut doc = None;
    let mut methods: Vec<*const PyMethodDef> = Vec::new();
    let mut execs: Vec<ExecSlot> = Vec::new();
    let mut has_abi = false;
    let mut multiple_interpreters_supported = true;

    for slot in PySlot::iter(slots) {
        match slot.as_kind(vm)? {
            PySlotKind::Module(PySlotModule::Abi(info)) => {
                info.is_supported(vm, Some(name))?;
                has_abi = true;
            }
            PySlotKind::Module(PySlotModule::Doc(text)) => doc = Some(text),
            PySlotKind::Module(PySlotModule::Methods(defs)) => methods.push(defs),
            PySlotKind::Module(PySlotModule::Exec(exec)) => execs.push(exec),
            PySlotKind::Module(PySlotModule::Create(_)) => {
                return Err(
                    vm.new_import_error("Py_mod_create is not supported yet", vm.ctx.new_str(name))
                );
            }
            PySlotKind::Module(PySlotModule::MultipleInterpreters(value)) => {
                // Py_MOD_MULTIPLE_INTERPRETERS_NOT_SUPPORTED is NULL.
                multiple_interpreters_supported = !value.is_null();
            }
            // The name comes from the import spec. The remaining slots are ignored for now.
            _ => {}
        }
    }

    if !has_abi {
        return Err(vm.new_import_error(
            "module does not define Py_mod_abi, which is mandatory for modules defined from slots",
            vm.ctx.new_str(name),
        ));
    }

    if !multiple_interpreters_supported
        && !vm.state.is_main_interpreter()
        && vm.state.feature_flags.check_multi_interp_extensions
    {
        return Err(vm.new_import_error(
            format!("module {name} does not support loading in subinterpreters"),
            vm.ctx.new_str(name),
        ));
    }

    let module = vm.new_module(
        name,
        vm.ctx.new_dict(),
        doc.map(|text| vm.ctx.new_str(text)),
    );
    let module_name = vm.ctx.new_str(name);
    for defs in methods {
        for def in PyMethodDef::iter(defs) {
            let function = def
                .build_method_def(vm)?
                .build_function(vm, Some(module.clone().into()));
            function
                .as_object()
                .set_attr("__module__", module_name.clone(), vm)?;
            let function_name = unsafe { def.ml_name.try_as_str(vm) }?;
            module.dict().set_item(function_name, function.into(), vm)?;
        }
    }

    if !execs.is_empty() {
        let pending = execs
            .into_iter()
            .map(|exec| {
                // Capsules cannot be created from Python, so Python code cannot forge a slot.
                PyCapsule::new(exec as *mut c_void, Some(PENDING_EXEC_NAME), None)
                    .into_ref(&vm.ctx)
                    .into()
            })
            .collect();
        module
            .dict()
            .set_item(PENDING_EXEC_KEY, vm.ctx.new_tuple(pending).into(), vm)?;
    }
    Ok(module.into())
}

/// Run the exec slots recorded by [`load_extension_module`], once.
fn exec_extension_module(vm: &VirtualMachine, module: &PyObject) -> PyResult<()> {
    let Some(module) = module.downcast_ref::<rustpython_vm::builtins::PyModule>() else {
        return Ok(());
    };
    let dict = module.dict();
    let Ok(pending) = dict.get_item(PENDING_EXEC_KEY, vm) else {
        return Ok(());
    };
    dict.del_item(PENDING_EXEC_KEY, vm)?;
    let module_ptr = module.as_object().as_raw().cast_mut();
    let pending = pending.downcast_ref::<PyTuple>().ok_or_else(|| {
        vm.new_system_error("corrupted pending exec slots of an extension module")
    })?;
    for slot in pending {
        let slot = slot
            .downcast_ref::<PyCapsule>()
            .filter(|slot| slot.name() == Some(PENDING_EXEC_NAME))
            .ok_or_else(|| {
                vm.new_system_error("corrupted pending exec slots of an extension module")
            })?;
        // Safety: the capsule was created by `load_extension_module` from an exec slot.
        let exec: ExecSlot = unsafe { core::mem::transmute(slot.pointer()) };
        if unsafe { exec(module_ptr) } != 0 {
            return Err(vm.take_raised_exception().unwrap_or_else(|| {
                vm.new_system_error("exec slot of an extension module failed without an exception")
            }));
        }
    }
    Ok(())
}
