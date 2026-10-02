#![allow(non_upper_case_globals, non_camel_case_types)]

use crate::PyObject;
use crate::pystate::with_vm;
use crate::util::{CStrExt, FfiPtrExt};
use alloc::ffi::CString;
use core::ffi::{c_char, c_int, c_void};
use rustpython_vm::protocol::{BufferFlags, PyBuffer};

pub const PyBUF_SIMPLE: c_int = 0;
pub const PyBUF_WRITABLE: c_int = 0x0001;
pub const PyBUF_WRITEABLE: c_int = PyBUF_WRITABLE;
pub const PyBUF_FORMAT: c_int = 0x0004;
pub const PyBUF_ND: c_int = 0x0008;
pub const PyBUF_STRIDES: c_int = 0x0010 | PyBUF_ND;
pub const PyBUF_C_CONTIGUOUS: c_int = 0x0020 | PyBUF_STRIDES;
pub const PyBUF_F_CONTIGUOUS: c_int = 0x0040 | PyBUF_STRIDES;
pub const PyBUF_ANY_CONTIGUOUS: c_int = 0x0080 | PyBUF_STRIDES;
pub const PyBUF_INDIRECT: c_int = 0x0100 | PyBUF_STRIDES;

pub const PyBUF_CONTIG: c_int = PyBUF_ND | PyBUF_WRITABLE;
pub const PyBUF_CONTIG_RO: c_int = PyBUF_ND;
pub const PyBUF_STRIDED: c_int = PyBUF_STRIDES | PyBUF_WRITABLE;
pub const PyBUF_STRIDED_RO: c_int = PyBUF_STRIDES;
pub const PyBUF_RECORDS: c_int = PyBUF_STRIDES | PyBUF_WRITABLE | PyBUF_FORMAT;
pub const PyBUF_RECORDS_RO: c_int = PyBUF_STRIDES | PyBUF_FORMAT;
pub const PyBUF_FULL: c_int = PyBUF_INDIRECT | PyBUF_WRITABLE | PyBUF_FORMAT;
pub const PyBUF_FULL_RO: c_int = PyBUF_INDIRECT | PyBUF_FORMAT;

pub const PyBUF_READ: c_int = 0x100;
pub const PyBUF_WRITE: c_int = 0x200;

#[repr(C)]
#[derive(Debug)]
pub struct Py_buffer {
    pub buf: *mut c_void,
    pub obj: *mut PyObject,
    pub len: isize,
    pub itemsize: isize,
    pub readonly: c_int,
    pub ndim: c_int,
    pub format: *mut c_char,
    pub shape: *mut isize,
    pub strides: *mut isize,
    pub suboffsets: *mut isize,
    pub internal: *mut c_void,
}

impl Default for Py_buffer {
    fn default() -> Self {
        Self {
            buf: core::ptr::null_mut(),
            obj: core::ptr::null_mut(),
            len: 0,
            itemsize: 0,
            readonly: 0,
            ndim: 0,
            format: core::ptr::null_mut(),
            shape: core::ptr::null_mut(),
            strides: core::ptr::null_mut(),
            suboffsets: core::ptr::null_mut(),
            internal: core::ptr::null_mut(),
        }
    }
}

struct PyBufferInternal {
    py_buf: PyBuffer,
    format_c_string: CString,
    shape: Vec<isize>,
    strides: Vec<isize>,
    suboffsets: Vec<isize>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CheckBuffer(obj: *mut PyObject) -> c_int {
    let Some(obj) = (unsafe { obj.assume_borrowed_or_opt() }) else {
        return 0;
    };
    obj.check_buffer() as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetBuffer(
    obj: *mut PyObject,
    view: *mut Py_buffer,
    flags: c_int,
) -> c_int {
    if view.is_null() {
        return with_vm(|vm| {
            vm.set_exception(Some(vm.new_system_error("NULL view in PyObject_GetBuffer")));
            -1
        });
    }

    with_vm(|vm| {
        let obj_ref = unsafe { obj.assume_borrowed_or_opt() }
            .ok_or_else(|| vm.new_system_error("NULL obj in PyObject_GetBuffer"))?;
        let buffer_flags = BufferFlags::from_bits_truncate(flags as u32);
        let py_buf = PyBuffer::from_object(vm, obj_ref, buffer_flags)?;

        let desc = &py_buf.desc;
        let ndim = desc.ndim();
        let py_buf_len = desc.len;
        let py_buf_itemsize = desc.itemsize;
        let readonly = desc.readonly;
        let has_suboffsets = desc.has_suboffsets();

        let shape: Vec<isize> = desc.dim_desc.iter().map(|(s, _, _)| *s as isize).collect();
        let strides: Vec<isize> = desc.dim_desc.iter().map(|(_, st, _)| *st).collect();
        let suboffsets: Vec<isize> = desc.dim_desc.iter().map(|(_, _, sub)| *sub).collect();
        let format_c_string = CString::new(desc.format.as_bytes()).unwrap_or_default();

        let buf_ptr = if readonly {
            py_buf
                .obj_bytes()
                .as_ptr()
                .wrapping_offset(desc.offset)
                .cast_mut()
                .cast()
        } else {
            py_buf
                .obj_bytes_mut()
                .as_mut_ptr()
                .wrapping_offset(desc.offset)
                .cast()
        };

        let internal = Box::new(PyBufferInternal {
            py_buf,
            format_c_string,
            shape,
            strides,
            suboffsets,
        });
        let internal_ptr = Box::into_raw(internal);

        unsafe {
            (*view).buf = buf_ptr;
            (*view).obj = obj_ref.to_owned().into_raw().as_ptr();
            (*view).len = py_buf_len as isize;
            (*view).itemsize = py_buf_itemsize as isize;
            (*view).readonly = c_int::from(readonly);
            (*view).ndim = ndim as c_int;
            (*view).format = if flags & PyBUF_FORMAT == PyBUF_FORMAT {
                (*internal_ptr).format_c_string.as_ptr().cast_mut()
            } else {
                core::ptr::null_mut()
            };
            (*view).shape = if flags & PyBUF_ND == PyBUF_ND {
                (*internal_ptr).shape.as_mut_ptr()
            } else {
                core::ptr::null_mut()
            };
            (*view).strides = if flags & PyBUF_STRIDES == PyBUF_STRIDES {
                (*internal_ptr).strides.as_mut_ptr()
            } else {
                core::ptr::null_mut()
            };
            (*view).suboffsets = if has_suboffsets && (flags & PyBUF_INDIRECT == PyBUF_INDIRECT) {
                (*internal_ptr).suboffsets.as_mut_ptr()
            } else {
                core::ptr::null_mut()
            };
            (*view).internal = internal_ptr.cast();
        }

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_Release(view: *mut Py_buffer) {
    if view.is_null() {
        return;
    }
    unsafe {
        if !(*view).internal.is_null() {
            let internal = Box::from_raw((*view).internal.cast::<PyBufferInternal>());
            internal.py_buf.release();
            drop(internal);
            (*view).internal = core::ptr::null_mut();
        }
        if !(*view).obj.is_null() {
            let _ = (*view).obj.assume_owned();
            (*view).obj = core::ptr::null_mut();
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_IsContiguous(view: *const Py_buffer, fort: c_char) -> c_int {
    if view.is_null() {
        return 0;
    }
    let view = unsafe { &*view };
    if !view.suboffsets.is_null() {
        return 0;
    }
    if view.ndim == 0 || view.len == 0 {
        return 1;
    }
    if view.strides.is_null() {
        return if fort == b'F' as c_char || fort == b'f' as c_char {
            i32::from(view.ndim == 1)
        } else {
            1
        };
    }

    let is_c = {
        let mut sd = view.itemsize;
        let mut c_contig = true;
        for i in (0..view.ndim as usize).rev() {
            let shape = unsafe { *view.shape.add(i) };
            let stride = unsafe { *view.strides.add(i) };
            if shape > 1 && stride != sd {
                c_contig = false;
                break;
            }
            sd *= shape;
        }
        c_contig
    };

    let is_f = {
        let mut sd = view.itemsize;
        let mut f_contig = true;
        for i in 0..view.ndim as usize {
            let shape = unsafe { *view.shape.add(i) };
            let stride = unsafe { *view.strides.add(i) };
            if shape > 1 && stride != sd {
                f_contig = false;
                break;
            }
            sd *= shape;
        }
        f_contig
    };

    match fort as u8 {
        b'C' | b'c' => i32::from(is_c),
        b'F' | b'f' => i32::from(is_f),
        b'A' | b'a' => i32::from(is_c || is_f),
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_FillContiguousStrides(
    ndim: c_int,
    shape: *mut isize,
    strides: *mut isize,
    itemsize: c_int,
    fort: c_char,
) {
    if strides.is_null() || shape.is_null() || ndim <= 0 {
        return;
    }
    let mut sd = itemsize as isize;
    if fort == b'F' as c_char || fort == b'f' as c_char {
        for i in 0..ndim as usize {
            unsafe { *strides.add(i) = sd };
            sd *= unsafe { *shape.add(i) };
        }
    } else {
        for i in (0..ndim as usize).rev() {
            unsafe { *strides.add(i) = sd };
            sd *= unsafe { *shape.add(i) };
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_FillInfo(
    view: *mut Py_buffer,
    o: *mut PyObject,
    buf: *mut c_void,
    len: isize,
    readonly: c_int,
    flags: c_int,
) -> c_int {
    if view.is_null() {
        return with_vm(|vm| {
            vm.set_exception(Some(vm.new_system_error("NULL view in PyBuffer_FillInfo")));
            -1
        });
    }

    if (flags & PyBUF_WRITABLE) == PyBUF_WRITABLE && readonly != 0 {
        return with_vm(|vm| {
            vm.set_exception(Some(
                vm.new_buffer_error("Object is not writable.".to_owned()),
            ));
            -1
        });
    }

    unsafe {
        (*view).buf = buf;
        (*view).obj = if let Some(obj) = o.assume_borrowed_or_opt() {
            obj.to_owned().into_raw().as_ptr()
        } else {
            core::ptr::null_mut()
        };
        (*view).len = len;
        (*view).itemsize = 1;
        (*view).readonly = readonly;
        (*view).ndim = 1;
        (*view).format = if (flags & PyBUF_FORMAT) == PyBUF_FORMAT {
            c"B".as_ptr().cast_mut()
        } else {
            core::ptr::null_mut()
        };
        (*view).shape = if (flags & PyBUF_ND) == PyBUF_ND {
            &raw mut (*view).len
        } else {
            core::ptr::null_mut()
        };
        (*view).strides = if (flags & PyBUF_STRIDES) == PyBUF_STRIDES {
            &raw mut (*view).itemsize
        } else {
            core::ptr::null_mut()
        };
        (*view).suboffsets = core::ptr::null_mut();
        (*view).internal = core::ptr::null_mut();
    }

    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_SizeFromFormat(format: *const c_char) -> isize {
    with_vm(|vm| {
        let fmt = unsafe { format.try_as_str(vm) }?;
        let struct_mod = vm.import("struct", 0)?;
        let calcsize = struct_mod.get_attr("calcsize", vm)?;
        let res = calcsize.call((fmt,), vm)?;
        let size: isize = res.try_index(vm)?.as_bigint().try_into().map_err(|_| {
            vm.new_overflow_error("struct.calcsize result too large for isize".to_owned())
        })?;
        Ok(size)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::prelude::*;
    use pyo3::types::PyBytes;

    #[test]
    fn buffer_get_and_release_bytes() {
        Python::attach(|py| {
            let bytes = PyBytes::new(py, b"hello buffer");
            let obj_ptr = bytes.as_ptr().cast();

            assert_eq!(unsafe { PyObject_CheckBuffer(obj_ptr) }, 1);

            let mut view = Py_buffer::default();
            let ret = unsafe { PyObject_GetBuffer(obj_ptr, &mut view, PyBUF_SIMPLE) };
            assert_eq!(ret, 0);
            assert_eq!(view.len, 12);
            assert_eq!(view.readonly, 1);
            assert!(!view.buf.is_null());

            let slice =
                unsafe { core::slice::from_raw_parts(view.buf.cast::<u8>(), view.len as usize) };
            assert_eq!(slice, b"hello buffer");

            assert_eq!(unsafe { PyBuffer_IsContiguous(&view, b'C' as c_char) }, 1);

            unsafe { PyBuffer_Release(&mut view) };
            assert!(view.obj.is_null());
        });
    }

    #[test]
    fn buffer_fill_info_and_size_from_format() {
        Python::attach(|_py| {
            let mut data = [1u8, 2, 3, 4];
            let mut view = Py_buffer::default();
            let ret = unsafe {
                PyBuffer_FillInfo(
                    &mut view,
                    core::ptr::null_mut(),
                    data.as_mut_ptr().cast(),
                    data.len() as isize,
                    0,
                    PyBUF_FULL,
                )
            };
            assert_eq!(ret, 0);
            assert_eq!(view.len, 4);
            assert_eq!(view.itemsize, 1);
            assert_eq!(view.readonly, 0);
            assert_eq!(view.ndim, 1);

            let size = unsafe { PyBuffer_SizeFromFormat(c"ii".as_ptr()) };
            assert_eq!(size, 8);

            let size_double = unsafe { PyBuffer_SizeFromFormat(c"d".as_ptr()) };
            assert_eq!(size_double, 8);

            let null_size = unsafe { PyBuffer_SizeFromFormat(core::ptr::null()) };
            assert_eq!(null_size, -1);
        });
    }

    #[test]
    fn buffer_fill_strides() {
        let mut shape = [2isize, 3isize, 4isize];
        let mut strides = [0isize; 3];
        unsafe {
            PyBuffer_FillContiguousStrides(
                3,
                shape.as_mut_ptr(),
                strides.as_mut_ptr(),
                4,
                b'C' as c_char,
            );
        }
        assert_eq!(strides, [48, 16, 4]);

        unsafe {
            PyBuffer_FillContiguousStrides(
                3,
                shape.as_mut_ptr(),
                strides.as_mut_ptr(),
                4,
                b'F' as c_char,
            );
        }
        assert_eq!(strides, [4, 8, 24]);
    }
}
