use crate::pystate::with_vm;
use core::ffi::c_int;
use rustpython_vm::Py;
use rustpython_vm::builtins::PyCode;
use rustpython_vm::frame::Frame;

pub type PyFrameObject = Py<Frame>;
pub type PyCodeObject = Py<PyCode>;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetCode(frame: *mut PyFrameObject) -> *mut PyCodeObject {
    with_vm(|_vm| Ok(unsafe { &*frame }.f_code()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetLineNumber(frame: *mut PyFrameObject) -> c_int {
    with_vm(|_vm| {
        let lineno = unsafe { &*frame }.f_lineno();
        Ok(lineno.try_into().unwrap_or(c_int::MAX))
    })
}
