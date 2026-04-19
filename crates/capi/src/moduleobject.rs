use crate::PyObject;
use crate::pystate::with_vm;
use rustpython_vm::builtins::PyModule;

#[repr(C)]
pub struct PyModuleDef {
    _private: [u8; 0],
}

#[unsafe(no_mangle)]
pub extern "C" fn PyModuleDef_Init(def: *mut PyModuleDef) -> *mut PyObject {
    def.cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyModule_GetNameObject(module: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let module = unsafe { &*module }.try_downcast_ref::<PyModule>(vm)?;
        module.get_attr("__name__", vm)
    })
}
