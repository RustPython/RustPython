use crate::object::define_py_check;
use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use core::ffi::{CStr, VaList, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use rustpython_vm::builtins::PyBytes;

define_py_check!(fn PyBytes_Check, types.bytes_type);
define_py_check!(exact fn PyBytes_CheckExact, types.bytes_type);

#[unsafe(no_mangle)]
#[allow(clippy::uninit_vec)]
pub unsafe extern "C" fn PyBytes_FromStringAndSize(
    bytes: *mut c_char,
    len: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let len = len.try_into().map_err(|_| {
            vm.new_system_error("Negative size passed to PyBytes_FromStringAndSize")
        })?;

        let data = if bytes.is_null() {
            let mut data = Vec::with_capacity(len);
            unsafe { data.set_len(len) };
            data
        } else {
            unsafe { core::slice::from_raw_parts(bytes as *const u8, len) }.to_vec()
        };

        Ok(vm.ctx.new_bytes(data))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_FromString(s: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let data = unsafe { CStr::from_ptr(s) }.to_bytes().to_vec();
        Ok(vm.ctx.new_bytes(data))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_FromFormat(format: *const c_char, args: ...) -> *mut PyObject {
    unsafe { PyBytes_FromFormatV(format, args) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_FromFormatV(
    format: *const c_char,
    mut args: VaList<'_>,
) -> *mut PyObject {
    with_vm(|vm| {
        let format = unsafe { CStr::from_ptr(format) }.to_bytes();
        let mut result = Vec::with_capacity(format.len());
        let mut pos = 0;
        while pos < format.len() {
            if format[pos] != b'%' {
                result.push(format[pos]);
                pos += 1;
                continue;
            }
            let start = pos;
            pos += 1;
            // The C API ignores field width and applies precision only to %s.
            while format.get(pos).is_some_and(u8::is_ascii_digit) {
                pos += 1;
            }
            let mut precision = 0usize;
            if format.get(pos) == Some(&b'.') {
                pos += 1;
                while let Some(&digit) = format.get(pos).filter(|b| b.is_ascii_digit()) {
                    precision = precision
                        .saturating_mul(10)
                        .saturating_add((digit - b'0').into());
                    pos += 1;
                }
            }
            while format
                .get(pos)
                .is_some_and(|b| *b != b'%' && !b.is_ascii_alphabetic())
            {
                pos += 1;
            }
            let modifier = if matches!(format.get(pos), Some(b'l' | b'z'))
                && matches!(format.get(pos + 1), Some(b'd' | b'u'))
            {
                let modifier = format[pos];
                pos += 1;
                Some(modifier)
            } else {
                None
            };
            match format.get(pos) {
                Some(b'c') => {
                    let value = unsafe { args.next_arg::<c_int>() };
                    let value = u8::try_from(value).map_err(|_| {
                        vm.new_overflow_error(
                            "PyBytes_FromFormatV(): %c format expects an integer in range [0; 255]",
                        )
                    })?;
                    result.push(value);
                }
                Some(b'd') => {
                    let value = match modifier {
                        Some(b'l') => unsafe { args.next_arg::<c_long>() }.to_string(),
                        Some(b'z') => unsafe { args.next_arg::<isize>() }.to_string(),
                        _ => unsafe { args.next_arg::<c_int>() }.to_string(),
                    };
                    result.extend_from_slice(value.as_bytes());
                }
                Some(b'u') => {
                    let value = match modifier {
                        Some(b'l') => unsafe { args.next_arg::<c_ulong>() }.to_string(),
                        Some(b'z') => unsafe { args.next_arg::<usize>() }.to_string(),
                        _ => unsafe { args.next_arg::<c_uint>() }.to_string(),
                    };
                    result.extend_from_slice(value.as_bytes());
                }
                Some(b'i') => result
                    .extend_from_slice(unsafe { args.next_arg::<c_int>() }.to_string().as_bytes()),
                Some(b'x') => result.extend_from_slice(
                    format!("{:x}", unsafe { args.next_arg::<c_uint>() }).as_bytes(),
                ),
                Some(b's') => {
                    let string = unsafe { args.next_arg::<*const c_char>() };
                    if precision == 0 {
                        result.extend_from_slice(unsafe { CStr::from_ptr(string) }.to_bytes());
                    } else {
                        // A precision-limited string need not have a terminator
                        // beyond the specified readable region.
                        for offset in 0..precision {
                            let byte = unsafe { *string.add(offset) } as u8;
                            if byte == 0 {
                                break;
                            }
                            result.push(byte);
                        }
                    }
                }
                Some(b'p') => {
                    let pointer = unsafe { args.next_arg::<*const c_void>() };
                    // Preserve the platform's %p spelling, including Windows
                    // uppercase digits, but always supply the C API's 0x prefix.
                    let mut buffer = [0 as c_char; 2 * size_of::<usize>() + 16];
                    unsafe {
                        libc::snprintf(buffer.as_mut_ptr(), buffer.len(), c"%p".as_ptr(), pointer);
                    }
                    let pointer = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_bytes();
                    result.extend_from_slice(b"0x");
                    result.extend_from_slice(
                        if pointer.starts_with(b"0x") || pointer.starts_with(b"0X") {
                            &pointer[2..]
                        } else {
                            pointer
                        },
                    );
                }
                Some(b'%') => result.push(b'%'),
                _ => {
                    // The first unrecognized format copies the remaining
                    // format verbatim, without consuming any more arguments.
                    result.extend_from_slice(&format[start..]);
                    break;
                }
            }
            pos += 1;
        }
        Ok(vm.ctx.new_bytes(result))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_FromObject(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        if let Some(bytes) = obj.downcast_ref::<PyBytes>() {
            Ok(bytes.to_owned())
        } else {
            obj.try_bytes_like(vm, |bytes| vm.ctx.new_bytes(bytes.to_vec()))
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_Size(bytes: *mut PyObject) -> isize {
    with_vm(|vm| {
        let bytes = unsafe { bytes.assume_borrowed_and_cast::<PyBytes>(vm) }?;
        Ok(bytes.as_bytes().len())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_AsString(bytes: *mut PyObject) -> *mut c_char {
    with_vm(|vm| {
        let bytes = unsafe { bytes.assume_borrowed_and_cast::<PyBytes>(vm) }?;
        Ok(bytes.as_bytes().as_ptr())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBytes_AsStringAndSize(
    obj: *mut PyObject,
    buffer: *mut *mut c_char,
    length: *mut isize,
) -> c_int {
    with_vm(|vm| {
        let data = unsafe { obj.assume_borrowed_and_cast::<PyBytes>(vm)? }.as_bytes();

        if length.is_null() {
            if data.contains(&0) {
                return Err(vm.new_value_error("embedded null byte"));
            }
        } else {
            unsafe { *length = data.len() as isize };
        };

        unsafe { *buffer = data.as_ptr().cast_mut().cast() };

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyBytes;

    #[test]
    fn bytes() {
        Python::attach(|py| {
            let bytes = PyBytes::new(py, b"Hello, World!");
            assert_eq!(bytes.as_bytes(), b"Hello, World!");
        })
    }

    #[test]
    fn bytes_uninit() {
        Python::attach(|py| {
            let bytes = PyBytes::new_with(py, 13, |data| {
                data.copy_from_slice(b"Hello, World!");
                Ok(())
            })
            .unwrap();
            assert_eq!(bytes.as_bytes(), b"Hello, World!");
        })
    }
}
