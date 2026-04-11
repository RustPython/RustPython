use crate::PyObject;
use crate::pystate::with_vm;
use rustpython_vm::builtins::PyModule;

#[unsafe(no_mangle)]
pub extern "C" fn PyModule_GetNameObject(module: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let module = unsafe { &*module }.try_downcast_ref::<PyModule>(vm)?;
        module.get_attr("__name__", vm)
    })
}
