//! Import of abi3t extension modules through their `PyModExport_<name>` entry point.

use crate::methodobject::PyMethodDef;
use crate::slots::{PySlot, PySlotKind, PySlotModule};
use crate::util::CStrExt;
use core::ffi::c_int;
use rustpython_vm::{AsObject, PyObject, PyResult, VirtualMachine};
use std::sync::Mutex;

type ExecSlot = unsafe extern "C" fn(*mut PyObject) -> c_int;

/// Exec slots of modules that are created but not yet executed, keyed by module address.
/// They run from `_imp.exec_dynamic`, once importlib has set the module attributes and added
/// the module to `sys.modules` (PEP 489).
static PENDING_EXEC: Mutex<Vec<(usize, Vec<ExecSlot>)>> = Mutex::new(Vec::new());

/// Make `_imp.create_dynamic` able to load extension modules built for the stable ABI.
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
        let key = module.as_object().as_raw() as usize;
        PENDING_EXEC
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((key, execs));
    }
    Ok(module.into())
}

/// Run the exec slots recorded by [`load_extension_module`], once.
fn exec_extension_module(vm: &VirtualMachine, module: &PyObject) -> PyResult<()> {
    let key = module.as_raw() as usize;
    let execs = {
        let mut pending = PENDING_EXEC
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match pending.iter().position(|(k, _)| *k == key) {
            Some(index) => pending.swap_remove(index).1,
            None => return Ok(()),
        }
    };
    let module_ptr = module.as_raw().cast_mut();
    for exec in execs {
        if unsafe { exec(module_ptr) } != 0 {
            return Err(vm.take_raised_exception().unwrap_or_else(|| {
                vm.new_system_error("exec slot of an extension module failed without an exception")
            }));
        }
    }
    Ok(())
}
