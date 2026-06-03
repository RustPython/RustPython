use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use rustpython_vm::builtins::{PyModule, PyStr};

define_py_check!(fn PyModule_Check, types.module_type);
define_py_check!(exact fn PyModule_CheckExact, types.module_type);

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
