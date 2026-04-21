use crate::handles::resolve_object_handle;
use crate::{PyObject, with_vm};
use core::ffi::c_char;
use rustpython_vm::builtins::PyByteArray;

#[unsafe(no_mangle)]
pub extern "C" fn PyByteArray_Size(bytearray: *mut PyObject) -> isize {
    with_vm(|vm| {
        let bytearray =
            unsafe { &*resolve_object_handle(bytearray) }.try_downcast_ref::<PyByteArray>(vm)?;
        Ok(bytearray.borrow_buf().len())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyByteArray_AsString(bytearray: *mut PyObject) -> *mut c_char {
    with_vm(|vm| {
        let bytearray =
            unsafe { &*resolve_object_handle(bytearray) }.try_downcast_ref::<PyByteArray>(vm)?;
        Ok(bytearray.borrow_buf().as_ptr())
    })
}
