use crate::PyObject;
use crate::methodobject::PyMethodDef;
use crate::pystate::with_vm;
use rustpython_vm::AsObject;
use rustpython_vm::builtins::PyModule;
use std::ffi::{CStr, c_char, c_int, c_void};

const PY_MOD_CREATE: c_int = 1;
const PY_MOD_EXEC: c_int = 2;
const PY_MOD_GIL: c_int = 4;

#[repr(C)]
pub struct PyModuleDef {
    m_base: [u8; 40],
    m_name: *const c_char,
    m_doc: *const c_char,
    m_size: isize,
    m_methods: *mut PyMethodDef,
    m_slots: *mut PyModuleDef_Slot,
    m_traverse: *mut c_void,
    m_clear: *mut c_void,
    m_free: *mut c_void,
}

#[repr(C)]
pub struct PyModuleDef_Slot {
    id: c_int,
    value: PyModuleDef_SlotValue,
}

union PyModuleDef_SlotValue {
    exec_module: extern "C" fn(*mut PyObject) -> c_int,
    gil_used: usize,
}

#[unsafe(no_mangle)]
pub extern "C" fn PyModuleDef_Init(def: *mut PyModuleDef) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { CStr::from_ptr((&*def).m_name) }
            .to_str()
            .expect("Module name is not valid UTF-8");
        let doc = unsafe { CStr::from_ptr((*def).m_doc) }
            .to_str()
            .expect("Module doc is not valid UTF-8");
        let dict = vm.ctx.new_dict();

        let module = vm.new_module(name, dict, Some(vm.ctx.new_str(doc)));

        let mut slot_ptr = unsafe { (*def).m_slots };
        while let slot = unsafe { &*slot_ptr }
            && slot.id != 0
        {
            match slot.id {
                PY_MOD_CREATE => {
                    return Err(vm.new_import_error(
                        "RustPython does not support modules that define a create slot",
                        vm.ctx.new_str(name),
                    ));
                }
                PY_MOD_EXEC => {
                    let exec_module = unsafe { slot.value.exec_module };
                    if exec_module(module.as_object().as_raw().cast_mut()) != 0 {
                        return Err(vm
                            .take_raised_exception()
                            .expect("Module exec function failed without setting an exception"));
                    }
                }
                PY_MOD_GIL => {
                    if unsafe { slot.value.gil_used == 0 } {
                        return Err(vm.new_import_error(
                            "RustPython does not support modules that require the GIL",
                            vm.ctx.new_str(name),
                        ));
                    }
                }
                _ => todo!("Got unknown PyModuleDef_Slot with id {}", slot.id),
            }

            slot_ptr = unsafe { slot_ptr.add(1) };
        }

        Ok(module)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyModule_GetNameObject(module: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let module = unsafe { &*module }.try_downcast_ref::<PyModule>(vm)?;
        module.get_attr("__name__", vm)
    })
}

#[cfg(test)]
mod tests {
    use super::PyModuleDef;
    use core::mem::offset_of;
    use pyo3::prelude::*;

    #[test]
    fn test_create_module() {
        const {
            assert!(
                offset_of!(PyModuleDef, m_name) == size_of::<pyo3::ffi::PyModuleDef_Base>(),
                "PyModuleDef::m_base was not the expected size"
            );
        };

        #[pymodule]
        mod my_extension {
            use pyo3::prelude::*;

            #[pymodule_export]
            const PI: f64 = std::f64::consts::PI;

            #[pyfunction] // Inline definition of a pyfunction, also made available to Python
            fn triple(x: usize) -> usize {
                x * 3
            }
        }

        Python::attach(|py| {
            let module = unsafe {
                Borrowed::from_ptr(py, my_extension::__pyo3_init()).cast_unchecked::<PyModule>()
            };
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
