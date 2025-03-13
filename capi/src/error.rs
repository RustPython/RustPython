use std::ffi;

use rustpython_vm::PyObjectRef;

#[unsafe(export_name = "PyErr_Clear")]
pub unsafe extern "C" fn err_clear() {
    todo!()
}

#[unsafe(export_name = "PyErr_NewException")]
pub unsafe extern "C" fn err_new_exception(
    name: *const ffi::c_char,
    _base: *mut ffi::c_void,
    _dict: *mut ffi::c_void,
) -> PyObjectRef {
    let vm = crate::get_vm();
    let name_str = unsafe { std::ffi::CStr::from_ptr(name).to_str().unwrap() };
    let name_split = name_str.split('.');
    let module = name_split.clone().next().unwrap();
    let name = name_split.last().unwrap();
    vm.ctx.new_exception_type(module, name, Some(vec![vm.ctx.exceptions.exception_type.to_owned()])).into()
}
