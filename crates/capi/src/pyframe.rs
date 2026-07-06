use crate::PyObject;
use crate::pystate::with_vm;
use rustpython_vm::frame::Frame;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetCode(frame: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let frame = unsafe { &*frame }.try_downcast_ref::<Frame>(vm)?;
        Ok(frame.code.clone())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetLineNumber(frame: *mut PyObject) -> core::ffi::c_int {
    with_vm(|vm| {
        let frame = unsafe { &*frame }.try_downcast_ref::<Frame>(vm)?;
        let lineno = if frame.lasti() == 0 {
            frame.code.first_line_number.map_or(1, |n| n.get())
        } else {
            frame.current_location().line.get()
        };
        Ok(lineno as core::ffi::c_int)
    })
}
