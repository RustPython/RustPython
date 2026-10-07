use crate::PyObject;
use crate::pystate::with_vm;
use crate::util::FfiPtrExt;
use alloc::borrow::Cow;
use alloc::ffi::CString;
use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use rustpython_vm::TryFromBorrowedObject;
use rustpython_vm::protocol::{BufferFlags, PyBuffer};

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

impl Py_buffer {
    fn is_contiguous_for_order(&self, order: u8) -> bool {
        let desc = unsafe { &(&*self.internal.cast::<BufferInternal>()).buffer.desc };
        if desc.len == 0 || desc.ndim() <= 1 {
            return true;
        }

        if desc.has_suboffsets() {
            return false;
        }

        match order {
            b'C' => desc.is_contiguous(),
            b'F' => desc.is_fortran_contiguous(),
            b'A' => desc.is_contiguous() || desc.is_fortran_contiguous(),
            _ => false,
        }
    }
}

#[allow(dead_code)]
struct BufferInternal {
    buffer: PyBuffer,
    format: Option<CString>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetBuffer(
    obj: *mut PyObject,
    view: *mut Py_buffer,
    flags: c_int,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let buffer = PyBuffer::try_from_borrowed_object(vm, obj)?;
        let flags = BufferFlags::from_bits(flags as u32)
            .ok_or_else(|| vm.new_system_error("invalid buffer flags"))?;

        if flags.contains(BufferFlags::WRITABLE) && buffer.desc.readonly {
            return Err(vm.new_buffer_error("Object is not writable"));
        }

        let format = flags
            .contains(BufferFlags::FORMAT)
            .then(|| {
                CString::new(&*buffer.desc.format)
                    .map_err(|_| vm.new_system_error("buffer format contains NUL"))
            })
            .transpose()?;

        let shape = if flags.contains(BufferFlags::ND) {
            buffer.desc.shape.as_ptr().cast_mut()
        } else {
            ptr::null_mut()
        };

        let strides = if flags.contains(BufferFlags::STRIDES) {
            buffer.desc.strides.as_ptr().cast_mut()
        } else {
            ptr::null_mut()
        };

        let suboffsets = if flags.contains(BufferFlags::INDIRECT) {
            buffer.desc.suboffsets.as_ptr().cast_mut()
        } else {
            ptr::null_mut()
        };

        let mut buffer_view = Py_buffer {
            len: buffer.desc.len as _,
            itemsize: buffer.desc.itemsize as _,
            readonly: buffer.desc.readonly.into(),
            ndim: buffer.desc.ndim() as _,
            format: format.as_ref().map_or_default(|s| s.as_ptr().cast_mut()),
            shape,
            strides,
            suboffsets,
            ..Default::default()
        };

        let c_contig = buffer.desc.is_contiguous();
        let f_contig = buffer.desc.is_fortran_contiguous();

        if flags.contains(BufferFlags::C_CONTIGUOUS) && !c_contig {
            return Err(vm.new_buffer_error("Object is not C-contiguous"));
        }
        if flags.contains(BufferFlags::F_CONTIGUOUS) && !f_contig {
            return Err(vm.new_buffer_error("Object is not Fortran-contiguous"));
        }
        if flags.contains(BufferFlags::ANY_CONTIGUOUS) && !(c_contig || f_contig) {
            return Err(vm.new_buffer_error("Object is not contiguous"));
        }

        buffer_view.obj = obj.as_raw().cast_mut();
        buffer_view.buf = buffer.obj_bytes().as_ptr().cast_mut().cast();
        buffer_view.internal = Box::into_raw(Box::new(BufferInternal { buffer, format })).cast();

        unsafe {
            ptr::replace(view, buffer_view);
        }
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_Release(view: *mut Py_buffer) {
    let view = unsafe { &mut *view };
    unsafe { drop(Box::from_raw(view.internal.cast::<BufferInternal>())) };
    core::mem::take(view);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_IsContiguous(view: *const Py_buffer, fort: c_char) -> c_int {
    unsafe { &*view }
        .is_contiguous_for_order((fort as u8).to_ascii_uppercase())
        .into()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_GetPointer(
    view: *const Py_buffer,
    indices: *const isize,
) -> *mut c_void {
    with_vm(|vm| {
        let view = unsafe { &*view };

        if view.strides.is_null() && view.shape.is_null() {
            let i0 = unsafe { *indices };
            let delta = i0
                .checked_mul(view.itemsize)
                .ok_or_else(|| vm.new_system_error("index * itemsize overflow"))?;
            return unsafe { Ok(view.buf.cast::<u8>().offset(delta).cast()) };
        }

        let ndim: usize = view
            .ndim
            .try_into()
            .map_err(|_| vm.new_system_error("view.ndim does not fit usize"))?;

        let idx = unsafe { core::slice::from_raw_parts(indices, ndim) };

        let strides: Cow<'_, _> = unsafe { view.strides.as_ref() }.map_or_else(
            || {
                let shape = unsafe { core::slice::from_raw_parts(view.shape, ndim) };
                let mut strides = vec![0; shape.len()];
                let mut stride = view.itemsize;
                for (ix, dim) in shape.iter().copied().enumerate().rev() {
                    strides[ix] = stride;
                    stride = stride
                        .checked_mul(dim)
                        .ok_or_else(|| vm.new_system_error("stride overflow"))?;
                }
                Ok(strides.into())
            },
            |strides| unsafe { Ok(core::slice::from_raw_parts(strides, ndim).into()) },
        )?;

        let suboffsets = unsafe {
            view.suboffsets
                .as_ref()
                .map(|suboffsets| core::slice::from_raw_parts(suboffsets, ndim))
        };

        let mut ptr_u8 = view.buf.cast::<u8>();
        for (dim, index) in idx.iter().enumerate() {
            let delta = index
                .checked_mul(strides[dim])
                .ok_or_else(|| vm.new_system_error("index * stride overflow"))?;
            ptr_u8 = unsafe { ptr_u8.offset(delta) };
            if let Some(suboffsets) = suboffsets {
                let suboffset = suboffsets[dim];
                if suboffset >= 0 {
                    let inner = unsafe { core::ptr::read_unaligned(ptr_u8.cast::<*mut u8>()) };
                    ptr_u8 = unsafe { inner.offset(suboffset) };
                }
            }
        }

        Ok(ptr_u8.cast())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyBuffer_ToContiguous(
    buf: *mut c_void,
    view: *const Py_buffer,
    len: isize,
    order: c_char,
) -> c_int {
    with_vm(|vm| {
        let view = unsafe { &*view };

        if !view.is_contiguous_for_order((order as u8).to_ascii_uppercase()) {
            return Err(vm.new_buffer_error(
                "PyBuffer_ToContiguous only supports contiguous exported buffers",
            ));
        }

        if len != view.len {
            return Err(vm.new_buffer_error("len must match view->len"));
        }

        let len: usize = len
            .try_into()
            .map_err(|_| vm.new_system_error("buffer len does not fit usize"))?;

        let src = unsafe { core::slice::from_raw_parts(view.buf.cast::<u8>(), len) };
        let dst = unsafe { core::slice::from_raw_parts_mut(buf.cast::<u8>(), len) };

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
        let view = unsafe { &*view };

        if view.readonly != 0 {
            return Err(vm.new_buffer_error("cannot write into readonly buffer"));
        }

        if !view.is_contiguous_for_order((order as u8).to_ascii_uppercase()) {
            return Err(vm.new_buffer_error(
                "PyBuffer_FromContiguous only supports contiguous exported buffers",
            ));
        }

        let len: usize = len
            .min(view.len)
            .try_into()
            .map_err(|_| vm.new_system_error("len does not fit usize"))?;

        let src = unsafe { core::slice::from_raw_parts(buf.cast::<u8>(), len) };
        let dst = unsafe { core::slice::from_raw_parts_mut(view.buf.cast::<u8>(), len) };
        dst.copy_from_slice(src);
        Ok(())
    })
}

#[cfg(test)]
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
