use crate::PyObject;
use crate::pystate::with_vm;
use crate::handles::resolve_object_handle;
use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use rustpython_vm::protocol::PyBuffer;
use rustpython_vm::TryFromBorrowedObject;

pub type Py_ssize_t = isize;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Py_buffer {
    pub buf: *mut c_void,
    pub obj: *mut PyObject,
    pub len: Py_ssize_t,
    pub itemsize: Py_ssize_t,
    pub readonly: c_int,
    pub ndim: c_int,
    pub format: *mut c_char,
    pub shape: *mut Py_ssize_t,
    pub strides: *mut Py_ssize_t,
    pub suboffsets: *mut Py_ssize_t,
    pub internal: *mut c_void,
}

struct BufferView {
    buffer: PyBuffer,
    format: Option<Vec<u8>>,
    shape: Vec<Py_ssize_t>,
    strides: Vec<Py_ssize_t>,
    suboffsets: Vec<Py_ssize_t>,
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GetBuffer(
    obj: *mut PyObject,
    view: *mut Py_buffer,
    _flags: c_int,
) -> c_int {
    with_vm(|vm| {
        let obj_ref = unsafe { &*resolve_object_handle(obj) };
        let buffer = rustpython_vm::protocol::PyBuffer::try_from_borrowed_object(vm, obj_ref)?;

        let mut owned = BufferView {
            format: Some(
                buffer
                    .desc
                    .format
                    .as_bytes()
                    .iter()
                    .copied()
                    .chain(core::iter::once(0))
                    .collect(),
            ),
            shape: buffer
                .desc
                .dim_desc
                .iter()
                .map(|(shape, _, _)| *shape as Py_ssize_t)
                .collect(),
            strides: buffer
                .desc
                .dim_desc
                .iter()
                .map(|(_, stride, _)| *stride as Py_ssize_t)
                .collect(),
            suboffsets: buffer
                .desc
                .dim_desc
                .iter()
                .map(|(_, _, suboffset)| *suboffset as Py_ssize_t)
                .collect(),
            buffer,
        };

        let buf_ptr = {
            let bytes = owned
                .buffer
                .as_contiguous()
                .ok_or_else(|| vm.new_buffer_error("non-contiguous buffers are not yet supported"))?;
            bytes.as_ptr().cast_mut().cast()
        };

        unsafe {
            (*view).buf = buf_ptr;
            (*view).obj = obj;
            (*view).len = owned.buffer.desc.len as Py_ssize_t;
            (*view).itemsize = owned.buffer.desc.itemsize as Py_ssize_t;
            (*view).readonly = owned.buffer.desc.readonly.into();
            (*view).ndim = owned.buffer.desc.ndim() as c_int;
            (*view).format = owned
                .format
                .as_mut()
                .map_or(ptr::null_mut(), |f| f.as_mut_ptr().cast());
            (*view).shape = if owned.shape.is_empty() {
                ptr::null_mut()
            } else {
                owned.shape.as_mut_ptr()
            };
            (*view).strides = if owned.strides.is_empty() {
                ptr::null_mut()
            } else {
                owned.strides.as_mut_ptr()
            };
            (*view).suboffsets = if owned.suboffsets.is_empty() {
                ptr::null_mut()
            } else {
                owned.suboffsets.as_mut_ptr()
            };
            (*view).internal = Box::into_raw(Box::new(owned)).cast();
        }
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyBuffer_Release(view: *mut Py_buffer) {
    if view.is_null() {
        return;
    }
    unsafe {
        if !(*view).internal.is_null() {
            drop(Box::from_raw((*view).internal.cast::<BufferView>()));
        }
        (*view).buf = ptr::null_mut();
        (*view).obj = ptr::null_mut();
        (*view).len = 0;
        (*view).itemsize = 0;
        (*view).readonly = 0;
        (*view).ndim = 0;
        (*view).format = ptr::null_mut();
        (*view).shape = ptr::null_mut();
        (*view).strides = ptr::null_mut();
        (*view).suboffsets = ptr::null_mut();
        (*view).internal = ptr::null_mut();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn PyBuffer_IsContiguous(view: *const Py_buffer, _fort: c_char) -> c_int {
    if view.is_null() {
        return 0;
    }
    unsafe {
        (*view)
            .internal
            .cast::<BufferView>()
            .as_ref()
            .is_some_and(|owned| owned.buffer.desc.is_contiguous()) as c_int
    }
}
