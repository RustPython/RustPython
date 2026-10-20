use crate::pystate::with_vm;
use crate::util::CStrExt;
use core::ffi::{c_char, c_int};
use core::ptr::NonNull;
use core::{slice, str};
use rustpython_vm::common::wtf8::{CodePoint, Wtf8Buf};
use rustpython_vm::{PyObject, PyResult, VirtualMachine};

pub struct PyUnicodeWriter {
    buffer: Wtf8Buf,
}

impl PyUnicodeWriter {
    fn try_reserve(&mut self, vm: &VirtualMachine, additional: usize) -> PyResult<()> {
        self.buffer
            .try_reserve(additional)
            .map_err(|_| vm.no_memory_error())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeWriter_Create(length: isize) -> *mut PyUnicodeWriter {
    with_vm(|vm| {
        let length = length
            .try_into()
            .map_err(|_| vm.new_value_error("invalid length"))?;

        let writer = Box::into_raw(Box::new(PyUnicodeWriter {
            buffer: Wtf8Buf::with_capacity(length),
        }));

        Ok(writer)
    })
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
        let codepoint =
            CodePoint::from_u32(ch).ok_or_else(|| vm.new_value_error("invalid char"))?;
        writer.try_reserve(vm, 4)?;
        writer.buffer.push(codepoint);
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
            let length = size
                .try_into()
                .map_err(|_| vm.new_value_error("invalid size"))?;
            let slice = unsafe { slice::from_raw_parts(str as *const u8, length) };
            str::from_utf8(slice).map_err(|_| vm.new_system_error("invalid UTF-8"))?
        };
        writer.try_reserve(vm, str.len())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::prelude::*;
    use pyo3::py_format;
    use pyo3::types::PyString;

    #[test]
    fn format_using_unicode_writer() {
        Python::attach(|py| {
            let name = String::from("world");
            let _string = py_format!(py, "Hello, {}!", name).unwrap();
        });
    }

    #[test]
    fn unicode_writer_raw() {
        Python::attach(|py| {
            let writer_ptr = unsafe { PyUnicodeWriter_Create(0) };
            assert!(!writer_ptr.is_null());

            let hello = "Hello, ";
            let world = "world!";
            let _ = unsafe {
                PyUnicodeWriter_WriteUTF8(
                    writer_ptr,
                    hello.as_ptr() as *const _,
                    hello.len() as isize,
                )
            };
            let _ = unsafe {
                PyUnicodeWriter_WriteUTF8(
                    writer_ptr,
                    world.as_ptr() as *const _,
                    world.len() as isize,
                )
            };
            assert_eq!(unsafe { PyUnicodeWriter_WriteChar(writer_ptr, 0xD800) }, 0);

            let string_ptr = unsafe { PyUnicodeWriter_Finish(writer_ptr) };
            assert!(!string_ptr.is_null());

            let string = unsafe {
                Bound::from_owned_ptr(py, string_ptr.cast())
                    .cast_into::<PyString>()
                    .unwrap()
            };
            assert_eq!(string.repr().unwrap(), "'Hello, world!\\ud800'");
        });
    }
}
