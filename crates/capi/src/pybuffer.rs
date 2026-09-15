use crate::PyObject;
use crate::pystate::with_vm;
use alloc::ffi::CString;
use core::ffi::{c_char, c_int, c_void};
use core::ptr::{self, NonNull};
use rustpython_vm::protocol::PyBuffer;
use rustpython_vm::{PyObjectRef, TryFromBorrowedObject};

const PYBUF_SIMPLE: c_int = 0;
const PYBUF_WRITABLE: c_int = 0x0001;
const PYBUF_FORMAT: c_int = 0x0004;
const PYBUF_ND: c_int = 0x0008;
const PYBUF_STRIDES: c_int = 0x0010 | PYBUF_ND;
const PYBUF_C_CONTIGUOUS: c_int = 0x0020 | PYBUF_STRIDES;
const PYBUF_F_CONTIGUOUS: c_int = 0x0040 | PYBUF_STRIDES;
const PYBUF_ANY_CONTIGUOUS: c_int = 0x0080 | PYBUF_STRIDES;
const PYBUF_INDIRECT: c_int = 0x0100 | PYBUF_STRIDES;

#[repr(C)]
#[derive(Default)]
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

struct BufferInternal {
    shape: Box<[isize]>,
    strides: Box<[isize]>,
    suboffsets: Box<[isize]>,
    format: CString,
}

fn is_contiguous_for_order(view: &Py_buffer, order: u8) -> bool {
    if view.len == 0 || view.ndim <= 1 {
        return true;
    }
    if view.shape.is_null() || view.strides.is_null() {
        return true;
    }

    let ndim: usize = match view.ndim.try_into() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let shape = unsafe { core::slice::from_raw_parts(view.shape, ndim) };
    let strides = unsafe { core::slice::from_raw_parts(view.strides, ndim) };
    if !view.suboffsets.is_null() {
        let suboffsets = unsafe { core::slice::from_raw_parts(view.suboffsets, ndim) };
        if suboffsets.iter().any(|&suboffset| suboffset >= 0) {
            return false;
        }
    }

    let check_c = || {
        let mut expected = view.itemsize;
        for i in (0..ndim).rev() {
            let dim = shape[i];
            if dim > 1 && strides[i] != expected {
                return false;
            }
            expected = match expected.checked_mul(dim) {
                Some(v) => v,
                None => return false,
            };
        }
        true
    };
    let check_f = || {
        let mut expected = view.itemsize;
        for i in 0..ndim {
            let dim = shape[i];
            if dim > 1 && strides[i] != expected {
                return false;
            }
            expected = match expected.checked_mul(dim) {
                Some(v) => v,
                None => return false,
            };
        }
        true
    };

    match order {
        b'C' => check_c(),
        b'F' => check_f(),
        b'A' => check_c() || check_f(),
        _ => false,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetBuffer(
    obj: *mut PyObject,
    view: *mut Py_buffer,
    flags: c_int,
) -> c_int {
    with_vm(|vm| {
        if view.is_null() {
            return Err(vm.new_system_error("PyObject_GetBuffer called with null view"));
        }

        let obj_ref = unsafe { &*obj };
        let buffer = PyBuffer::try_from_borrowed_object(vm, obj_ref)?;

        if (flags & PYBUF_WRITABLE) != 0 && buffer.desc.readonly {
            return Err(vm.new_buffer_error("Object is not writable"));
        }

        let ndim = buffer.desc.ndim();
        let ndim_i32: c_int = ndim
            .try_into()
            .map_err(|_| vm.new_system_error("buffer ndim does not fit c_int"))?;
        let len: isize = buffer
            .desc
            .len
            .try_into()
            .map_err(|_| vm.new_system_error("buffer len does not fit isize"))?;
        let itemsize: isize = buffer
            .desc
            .itemsize
            .try_into()
            .map_err(|_| vm.new_system_error("buffer itemsize does not fit isize"))?;

        let shape: Vec<isize> = buffer
            .desc
            .dim_desc
            .iter()
            .map(|(dim, _, _)| {
                (*dim)
                    .try_into()
                    .map_err(|_| vm.new_system_error("buffer shape does not fit isize"))
            })
            .collect::<Result<_, _>>()?;
        let strides: Vec<isize> = buffer
            .desc
            .dim_desc
            .iter()
            .map(|(_, stride, _)| *stride)
            .collect();
        let suboffsets: Vec<isize> = buffer
            .desc
            .dim_desc
            .iter()
            .map(|(_, _, suboffset)| if *suboffset == 0 { -1 } else { *suboffset })
            .collect();

        let contig_view = Py_buffer {
            buf: ptr::null_mut(),
            obj: ptr::null_mut(),
            len,
            itemsize,
            readonly: 0,
            ndim: ndim_i32,
            format: ptr::null_mut(),
            shape: shape.as_ptr().cast_mut(),
            strides: strides.as_ptr().cast_mut(),
            suboffsets: suboffsets.as_ptr().cast_mut(),
            internal: ptr::null_mut(),
        };
        let c_contig = is_contiguous_for_order(&contig_view, b'C');
        let f_contig = is_contiguous_for_order(&contig_view, b'F');

        if (flags & !PYBUF_WRITABLE) == PYBUF_SIMPLE && !c_contig {
            return Err(vm.new_buffer_error("Object is not C-contiguous for PyBUF_SIMPLE request"));
        }

        if (flags & PYBUF_C_CONTIGUOUS) == PYBUF_C_CONTIGUOUS && !c_contig {
            return Err(vm.new_buffer_error("Object is not C-contiguous"));
        }
        if (flags & PYBUF_F_CONTIGUOUS) == PYBUF_F_CONTIGUOUS && !f_contig {
            return Err(vm.new_buffer_error("Object is not Fortran-contiguous"));
        }
        if (flags & PYBUF_ANY_CONTIGUOUS) == PYBUF_ANY_CONTIGUOUS && !(c_contig || f_contig) {
            return Err(vm.new_buffer_error("Object is not contiguous"));
        }

        let format = CString::new(&*buffer.desc.format)
            .map_err(|_| vm.new_system_error("buffer format contains NUL"))?;

        let mut internal = Box::new(BufferInternal {
            shape: shape.into_boxed_slice(),
            strides: strides.into_boxed_slice(),
            suboffsets: suboffsets.into_boxed_slice(),
            format,
        });

        let view_ref = unsafe { &mut *view };
        view_ref.buf = buffer.obj_bytes().as_ptr().cast_mut().cast();
        view_ref.obj = obj_ref.to_owned().into_raw().as_ptr().cast();
        view_ref.len = len;
        view_ref.itemsize = itemsize;
        view_ref.readonly = c_int::from(buffer.desc.readonly);
        view_ref.ndim = ndim_i32;
        view_ref.format = if (flags & PYBUF_FORMAT) != 0 {
            internal.format.as_ptr().cast_mut()
        } else {
            ptr::null_mut()
        };
        view_ref.shape = if (flags & PYBUF_ND) != 0 {
            internal.shape.as_mut_ptr()
        } else {
            ptr::null_mut()
        };
        view_ref.strides = if (flags & PYBUF_STRIDES) != 0 {
            internal.strides.as_mut_ptr()
        } else {
            ptr::null_mut()
        };
        view_ref.suboffsets = if (flags & PYBUF_INDIRECT) != 0 {
            internal.suboffsets.as_mut_ptr()
        } else {
            ptr::null_mut()
        };
        view_ref.internal = Box::into_raw(internal).cast();
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_Release(view: *mut Py_buffer) {
    if view.is_null() {
        return;
    }
    let view_ref = unsafe { &mut *view };

    if let Some(obj) = NonNull::new(view_ref.obj) {
        unsafe { drop(PyObjectRef::from_raw(obj)) };
    }
    if let Some(internal) = NonNull::new(view_ref.internal.cast::<BufferInternal>()) {
        unsafe { drop(Box::from_raw(internal.as_ptr())) };
    }

    *view_ref = Py_buffer::default();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_IsContiguous(view: *const Py_buffer, fort: c_char) -> c_int {
    let Some(view_ref) = (unsafe { view.as_ref() }) else {
        return 0;
    };
    is_contiguous_for_order(view_ref, (fort as u8).to_ascii_uppercase()).into()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_GetPointer(
    view: *const Py_buffer,
    indices: *const isize,
) -> *mut c_void {
    let Some(view_ref) = (unsafe { view.as_ref() }) else {
        return ptr::null_mut();
    };
    if indices.is_null() {
        return ptr::null_mut();
    }
    let ndim: usize = match view_ref.ndim.try_into() {
        Ok(v) => v,
        Err(_) => return ptr::null_mut(),
    };
    let idx = unsafe { core::slice::from_raw_parts(indices, ndim) };
    let synthetic_strides = if !view_ref.strides.is_null() {
        None
    } else if !view_ref.shape.is_null() {
        let shape = unsafe { core::slice::from_raw_parts(view_ref.shape, ndim) };
        let mut strides = vec![0; shape.len()];
        let mut stride = view_ref.itemsize;
        for (ix, dim) in shape.iter().copied().enumerate().rev() {
            strides[ix] = stride;
            stride = match stride.checked_mul(dim) {
                Some(v) => v,
                None => return ptr::null_mut(),
            };
        }
        Some(strides)
    } else {
        let i0 = unsafe { *indices };
        let delta = match i0.checked_mul(view_ref.itemsize) {
            Some(v) => v,
            None => return ptr::null_mut(),
        };
        let base = view_ref.buf.cast::<u8>();
        if base.is_null() {
            return ptr::null_mut();
        }
        return unsafe { base.offset(delta).cast() };
    };
    let strides: &[isize] = if let Some(strides) = synthetic_strides.as_deref() {
        strides
    } else {
        unsafe { core::slice::from_raw_parts(view_ref.strides, ndim) }
    };

    let suboffsets = if view_ref.suboffsets.is_null() {
        None
    } else {
        Some(unsafe { core::slice::from_raw_parts(view_ref.suboffsets, ndim) })
    };

    let mut ptr_u8 = view_ref.buf.cast::<u8>();
    if ptr_u8.is_null() {
        return ptr::null_mut();
    }
    for (dim, index) in idx.iter().copied().enumerate() {
        let delta = match index.checked_mul(strides[dim]) {
            Some(v) => v,
            None => return ptr::null_mut(),
        };
        ptr_u8 = unsafe { ptr_u8.offset(delta) };
        if let Some(suboffsets) = suboffsets {
            let suboffset = suboffsets[dim];
            if suboffset >= 0 {
                let inner = unsafe { core::ptr::read_unaligned(ptr_u8.cast::<*mut u8>()) };
                if inner.is_null() {
                    return ptr::null_mut();
                }
                ptr_u8 = unsafe { inner.offset(suboffset) };
            }
        }
    }

    ptr_u8.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_ToContiguous(
    buf: *mut c_void,
    view: *const Py_buffer,
    len: isize,
    order: c_char,
) -> c_int {
    with_vm(|vm| {
        if buf.is_null() {
            return Err(vm.new_system_error("PyBuffer_ToContiguous called with null destination"));
        }
        let view_ref = unsafe { view.as_ref() }
            .ok_or_else(|| vm.new_system_error("PyBuffer_ToContiguous called with null view"))?;
        if len < 0 {
            return Err(vm.new_system_error("PyBuffer_ToContiguous called with negative len"));
        }
        if !is_contiguous_for_order(view_ref, (order as u8).to_ascii_uppercase()) {
            return Err(vm.new_buffer_error(
                "PyBuffer_ToContiguous only supports contiguous exported buffers",
            ));
        }

        let have: usize = view_ref
            .len
            .try_into()
            .map_err(|_| vm.new_system_error("buffer len does not fit usize"))?;
        if usize::try_from(len).map_err(|_| vm.new_system_error("len does not fit usize"))? != have
        {
            return Err(vm.new_buffer_error("len must match view->len"));
        }
        let src = unsafe { core::slice::from_raw_parts(view_ref.buf.cast::<u8>(), have) };
        let dst = unsafe { core::slice::from_raw_parts_mut(buf.cast::<u8>(), have) };
        dst.copy_from_slice(src);
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_FromContiguous(
    view: *const Py_buffer,
    buf: *const c_void,
    len: isize,
    order: c_char,
) -> c_int {
    with_vm(|vm| {
        if buf.is_null() {
            return Err(vm.new_system_error("PyBuffer_FromContiguous called with null source"));
        }
        let view_ref = unsafe { view.as_ref() }
            .ok_or_else(|| vm.new_system_error("PyBuffer_FromContiguous called with null view"))?;
        if view_ref.readonly != 0 {
            return Err(vm.new_buffer_error("cannot write into readonly buffer"));
        }
        if len < 0 {
            return Err(vm.new_system_error("PyBuffer_FromContiguous called with negative len"));
        }
        if !is_contiguous_for_order(view_ref, (order as u8).to_ascii_uppercase()) {
            return Err(vm.new_buffer_error(
                "PyBuffer_FromContiguous only supports contiguous exported buffers",
            ));
        }

        let have: usize = view_ref
            .len
            .try_into()
            .map_err(|_| vm.new_system_error("buffer len does not fit usize"))?;
        if usize::try_from(len).map_err(|_| vm.new_system_error("len does not fit usize"))? != have
        {
            return Err(vm.new_buffer_error("len must match view->len"));
        }
        let src = unsafe { core::slice::from_raw_parts(buf.cast::<u8>(), have) };
        let dst = unsafe { core::slice::from_raw_parts_mut(view_ref.buf.cast::<u8>(), have) };
        dst.copy_from_slice(src);
        Ok(())
    })
}

#[cfg(false)]
mod tests {
    use pyo3::buffer::PyBuffer;
    use pyo3::prelude::*;
    use pyo3::types::{PyByteArray, PyBytes};

    #[test]
    fn object_getbuffer_basic_and_release() {
        Python::attach(|py| {
            let bytes = PyBytes::new(py, b"hello");
            let buffer = PyBuffer::<u8>::get(&bytes).unwrap();
            assert_eq!(buffer.dimensions(), 1);
            assert_eq!(buffer.item_count(), 5);
            assert_eq!(buffer.to_vec(py).unwrap(), b"hello");
        });
    }

    #[test]
    fn contiguous_copy_roundtrip() {
        Python::attach(|py| {
            let src = PyBytes::new(py, b"abcde");
            let buffer = PyBuffer::<u8>::get(&src).unwrap();
            let mut out = [0u8; 5];
            buffer.copy_to_slice(py, &mut out).unwrap();
            assert_eq!(&out, b"abcde");
        });
    }

    #[test]
    fn is_contiguous_and_get_pointer() {
        Python::attach(|py| {
            let bytes = PyBytes::new(py, b"xyz");
            let buffer = PyBuffer::<u8>::get(&bytes).unwrap();
            assert!(buffer.is_c_contiguous());

            let p = buffer.get_ptr(&[1]);
            assert!(!p.is_null());
            unsafe { assert_eq!(*(p.cast::<u8>()), b'y') };
        });
    }

    #[test]
    fn writable_bytearray() {
        Python::attach(|py| {
            let bytearray = PyByteArray::new(py, b"hello");
            let buffer = PyBuffer::<u8>::get(&bytearray).unwrap();
            assert_eq!(buffer.dimensions(), 1);
            assert_eq!(buffer.item_count(), 5);
            buffer.as_mut_slice(py).unwrap()[0].replace(b'H');
            drop(buffer);
            assert_eq!(bytearray.to_vec(), b"Hello");
        });
    }
}
