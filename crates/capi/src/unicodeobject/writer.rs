use crate::pystate::with_vm;
use crate::util::CStrExt;
use core::ffi::{c_char, c_int};
use core::ptr::NonNull;
use core::{slice, str};
use rustpython_vm::PyObject;

pub struct PyUnicodeWriter {
    buffer: String,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeWriter_Create(length: isize) -> *mut PyUnicodeWriter {
    Box::into_raw(Box::new(PyUnicodeWriter {
        buffer: String::with_capacity(length as usize),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeWriter_Finish(writer: *mut PyUnicodeWriter) -> *mut PyObject {
    with_vm(|vm| {
        let writer = unsafe { Box::from_raw(writer) };
        Ok(vm.ctx.new_str(writer.buffer))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeWriter_Discard(writer: *mut PyUnicodeWriter) {
    if let Some(ptr) = NonNull::new(writer) {
        let _ = unsafe { Box::from_non_null(ptr) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeWriter_WriteChar(writer: *mut PyUnicodeWriter, ch: u32) -> c_int {
    with_vm(|vm| {
        let writer = unsafe { &mut *writer };
        let char = ch
            .try_into()
            .map_err(|_| vm.new_value_error("invalid char"))?;
        writer.buffer.push(char);
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeWriter_WriteUTF8(
    writer: *mut PyUnicodeWriter,
    str: *const c_char,
    size: isize,
) -> c_int {
    with_vm(|vm| {
        let writer = unsafe { &mut *writer };
        let str = if size == -1 {
            unsafe { str.try_as_str(vm)? }
        } else {
            let slice = unsafe { slice::from_raw_parts(str as *const u8, size as usize) };
            str::from_utf8(slice).map_err(|_| vm.new_system_error("invalid UTF-8"))?
        };
        writer.buffer.push_str(str);
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeWriter_WriteASCII(
    writer: *mut PyUnicodeWriter,
    str: *const c_char,
    size: isize,
) -> c_int {
    unsafe { PyUnicodeWriter_WriteUTF8(writer, str, size) }
}
