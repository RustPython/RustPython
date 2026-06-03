use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::c_int;
use rustpython_vm::PyPayload;
use rustpython_vm::builtins::PySlice;
use rustpython_vm::sliceable::SaturatedSlice;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_New(
    start: *mut PyObject,
    stop: *mut PyObject,
    step: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let start = if start.is_null() {
            None
        } else {
            Some(unsafe { &*start }.to_owned())
        };
        let stop = if stop.is_null() {
            vm.ctx.none()
        } else {
            unsafe { &*stop }.to_owned()
        };
        let step = if step.is_null() {
            None
        } else {
            Some(unsafe { &*step }.to_owned())
        };
        Ok(PySlice { start, stop, step }.into_ref(&vm.ctx))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_Unpack(
    slice: *mut PyObject,
    start: *mut isize,
    stop: *mut isize,
    step: *mut isize,
) -> c_int {
    with_vm(|vm| {
        let slice = unsafe { &*slice }.try_downcast_ref::<PySlice>(vm)?;
        let saturated = slice.to_saturated(vm)?;
        unsafe {
            *start = saturated.start();
            *stop = saturated.stop();
            *step = saturated.step();
        }
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_AdjustIndices(
    length: isize,
    start: *mut isize,
    stop: *mut isize,
    step: isize,
) -> isize {
    let length = length.max(0) as usize;
    let saturated = SaturatedSlice::from_parts(unsafe { *start }, unsafe { *stop }, step);
    let (range, _, slice_len) = saturated.adjust_indices(length);
    unsafe {
        if step.is_negative() {
            *start = range.end as isize - 1;
            *stop = range.start as isize - 1;
        } else {
            *start = range.start as isize;
            *stop = range.end as isize;
        }
    }
    slice_len as isize
}

#[cfg(false)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PySlice, PySliceMethods};

    #[test]
    fn slice_new_indices() {
        Python::attach(|py| {
            let slice = PySlice::new(py, 1, 10, 3);
            let indices = slice.indices(100).unwrap();
            assert_eq!((indices.start, indices.stop, indices.step), (1, 10, 3));
        })
    }

    #[test]
    fn slice_full_defaults() {
        Python::attach(|py| {
            let slice = PySlice::full(py);
            let indices = slice.indices(5).unwrap();
            assert_eq!((indices.start, indices.stop, indices.step), (0, 5, 1));
        })
    }

    #[test]
    fn slice_new_negative_step() {
        Python::attach(|py| {
            let slice = PySlice::new(py, 10, 1, -2);
            let indices = slice.indices(100).unwrap();
            assert_eq!((indices.start, indices.stop, indices.step), (10, 1, -2));
        })
    }
}
