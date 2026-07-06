use crate::PyObject;
use crate::pystate::with_vm;
use rustpython_vm::frame::Frame;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetCode(frame: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let frame = unsafe { &*frame }.try_downcast_ref::<Frame>(vm)?;
        Ok(frame.f_code())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetLineNumber(frame: *mut PyObject) -> core::ffi::c_int {
    with_vm(|vm| {
        let frame = unsafe { &*frame }.try_downcast_ref::<Frame>(vm)?;
        Ok(frame.f_lineno() as core::ffi::c_int)
    })
}
