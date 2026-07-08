use crate::PyObject;
use crate::pystate::with_vm;
use rustpython_vm::Py;
use rustpython_vm::frame::Frame;

pub type PyFrameObject = Py<Frame>;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetCode(frame: *mut PyFrameObject) -> *mut PyObject {
    with_vm(|_vm| Ok(unsafe { &*frame }.f_code()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetLineNumber(frame: *mut PyFrameObject) -> core::ffi::c_int {
    with_vm(|_vm| Ok(unsafe { &*frame }.f_lineno() as core::ffi::c_int))
}
