use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::slots::{PySlot, PySlotKind, PySlotModule};
use core::ffi::c_int;
use rustpython_vm::builtins::{PyModule, PyModuleDef, PyStr};

define_py_check!(fn PyModule_Check, types.module_type);
define_py_check!(exact fn PyModule_CheckExact, types.module_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_FromSlotsAndSpec(
    slots: *const PySlot,
    spec: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { &*spec }
            .get_attr("name", vm)?
            .downcast_exact::<PyStr>(vm)
            .unwrap();

        let mut exec = None;
        let mut create = None;

        for slot in PySlot::iter(slots) {
            match slot.as_kind(vm)? {
                PySlotKind::Module(module) => match module {
                    PySlotModule::Create(mod_create) => create = Some(mod_create),
                    PySlotModule::Exec(mod_exec) => {
                        if exec.replace(mod_exec).is_some() {
                            return Err(vm.new_system_error("Multiple module exec slots found"));
                        }
                    }
                    PySlotModule::Name { .. }
                    | PySlotModule::Doc { .. }
                    | PySlotModule::Methods(_)
                    | PySlotModule::Abi { .. }
                    | PySlotModule::MultipleInterpreters { .. }
                    | PySlotModule::Gil { .. } => {}
                },
                kind @ PySlotKind::Type(_) => {
                    return Err(vm.new_system_error(format!(
                        "Got type slot while module slots are expected: {kind:?}"
                    )));
                }
                PySlotKind::Unknown { .. } => {}
            }
        }

        let def = PyModuleDef::from_slots(vm.ctx.intern_str(name), None, create, exec);

        def.create_module_owned(vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_Exec(module: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let module = unsafe { &*module }.try_downcast_ref::<PyModule>(vm)?;
        let def = module
            .def
            .as_deref()
            .ok_or_else(|| vm.new_system_error("Empty module"))?;
        def.exec_module(vm, module)?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetNameObject(module: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let module = unsafe { &*module }.try_downcast_ref::<PyModule>(vm)?;
        let dict = module.dict();
        let name = dict
            .get_item_opt(rustpython_vm::identifier!(vm, __name__), vm)?
            .and_then(|obj| obj.downcast_ref::<PyStr>().map(ToOwned::to_owned));
        name.ok_or_else(|| vm.new_system_error("nameless module"))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_GetFilenameObject(module: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let module = unsafe { &*module }.try_downcast_ref::<PyModule>(vm)?;
        let dict = module.dict();
        let filename = dict
            .get_item_opt(rustpython_vm::identifier!(vm, __file__), vm)?
            .and_then(|obj| obj.downcast_ref::<PyStr>().map(ToOwned::to_owned));
        filename.ok_or_else(|| vm.new_system_error("module filename missing"))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyModule_NewObject(name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| -> rustpython_vm::PyResult<_> {
        let name = unsafe { &*name }.try_downcast_ref::<PyStr>(vm)?;
        let name = name
            .to_str()
            .ok_or_else(|| vm.new_system_error("module name must be valid UTF-8"))?;
        Ok(vm.new_module(name, vm.ctx.new_dict(), None))
    })
}

#[cfg(test)]
mod tests {
    use pyo3::ffi;
    use pyo3::prelude::*;

    #[test]
    fn create_module() {
        #[pymodule]
        mod my_extension {
            use pyo3::prelude::*;

            #[pymodule_export]
            const PI: f64 = core::f64::consts::PI;

            #[pyfunction] // Inline definition of a pyfunction, also made available to Python
            fn triple(x: usize) -> usize {
                x * 3
            }
        }

        fn create_module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
            let spec = py.import("types")?.getattr("SimpleNamespace")?.call0()?;
            spec.setattr("name", "my_extension")?;
            let slots = unsafe { my_extension::__pyo3_export() };
            let module = unsafe {
                Bound::from_owned_ptr_or_err(
                    py,
                    ffi::PyModule_FromSlotsAndSpec(slots, spec.as_ptr()),
                )?
                .cast_into_unchecked::<PyModule>()
            };
            unsafe { ffi::PyModule_Exec(module.as_ptr()) };
            Ok(module)
        }

        Python::attach(|py| {
            let module = create_module(py).unwrap();
            assert_eq!(module.name().unwrap(), "my_extension");

            module.getattr("PI").unwrap().extract::<f64>().unwrap();

            assert_eq!(
                module
                    .getattr("triple")
                    .unwrap()
                    .call1((10,))
                    .unwrap()
                    .extract::<usize>()
                    .unwrap(),
                30
            );
        })
    }
}
