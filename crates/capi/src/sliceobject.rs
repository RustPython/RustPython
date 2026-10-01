use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::util::FfiPtrExt;
use core::ffi::c_int;
use rustpython_vm::PyPayload;
use rustpython_vm::builtins::PySlice;
use rustpython_vm::sliceable::SaturatedSlice;

define_py_check!(fn PySlice_Check, types.slice_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_New(
    start: *mut PyObject,
    stop: *mut PyObject,
    step: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let start = unsafe { start.assume_borrowed_or_opt() }.map(ToOwned::to_owned);
        let stop = unsafe { stop.assume_borrowed_or_opt() }
            .map_or_else(|| vm.ctx.none(), ToOwned::to_owned);
        let step = unsafe { step.assume_borrowed_or_opt() }.map(ToOwned::to_owned);
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
        let slice = unsafe { slice.assume_borrowed_and_cast::<PySlice>(vm) }?;
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_GetIndices(
    r: *mut PyObject,
    length: isize,
    start: *mut isize,
    stop: *mut isize,
    step: *mut isize,
) -> c_int {
    with_vm(|vm| {
        let slice = unsafe { r.assume_borrowed_and_cast::<PySlice>(vm) }?;
        let step_val = if let Some(step) = &slice.step
            && !vm.is_none(step)
        {
            let s: isize = step
                .try_index(vm)?
                .as_bigint()
                .try_into()
                .map_err(|_| vm.new_value_error("step out of range".to_owned()))?;
            if s == 0 {
                return Ok(-1);
            }
            s
        } else {
            1
        };
        let start_val = if let Some(start) = &slice.start
            && !vm.is_none(start)
        {
            let mut st: isize = start
                .try_index(vm)?
                .as_bigint()
                .try_into()
                .map_err(|_| vm.new_value_error("start out of range".to_owned()))?;
            if st < 0 {
                st += length;
            }
            st
        } else if step_val < 0 {
            length - 1
        } else {
            0
        };
        let stop_val = if !vm.is_none(&slice.stop) {
            let mut sp: isize = slice
                .stop
                .try_index(vm)?
                .as_bigint()
                .try_into()
                .map_err(|_| vm.new_value_error("stop out of range".to_owned()))?;
            if sp < 0 {
                sp += length;
            }
            sp
        } else if step_val < 0 {
            -1
        } else {
            length
        };
        if !start.is_null() {
            unsafe { *start = start_val };
        }
        if !stop.is_null() {
            unsafe { *stop = stop_val };
        }
        if !step.is_null() {
            unsafe { *step = step_val };
        }

        if stop_val > length || start_val >= length {
            return Ok(-1);
        }
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySlice_GetIndicesEx(
    r: *mut PyObject,
    length: isize,
    start: *mut isize,
    stop: *mut isize,
    step: *mut isize,
    slicelength: *mut isize,
) -> c_int {
    if unsafe { PySlice_Unpack(r, start, stop, step) } != 0 {
        return -1;
    }
    let len = unsafe { PySlice_AdjustIndices(length, start, stop, *step) };
    if !slicelength.is_null() {
        unsafe { *slicelength = len };
    }
    0
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyInt, PySlice, PySliceMethods};

    #[test]
    fn slice_check() {
        Python::attach(|py| {
            let slice = PySlice::new(py, 0, 5, 1);
            let not_a_slice = PyInt::new(py, 42);
            unsafe {
                assert_ne!(super::PySlice_Check(slice.as_ptr().cast()), 0);
                assert_eq!(super::PySlice_Check(not_a_slice.as_ptr().cast()), 0);
            }
        });
    }

    #[test]
    fn slice_new_indices() {
        Python::attach(|py| {
            let slice = PySlice::new(py, 1, 10, 3);
            let indices = slice.indices(100).unwrap();
            assert_eq!((indices.start, indices.stop, indices.step), (1, 10, 3));
        })
    }

    #[test]
    fn slice_new_c_api() {
        Python::attach(|py| {
            let start = PyInt::new(py, 2);
            let stop = PyInt::new(py, 8);
            let step = PyInt::new(py, 2);
            let slice_ptr = unsafe {
                super::PySlice_New(
                    start.as_ptr().cast(),
                    stop.as_ptr().cast(),
                    step.as_ptr().cast(),
                )
            };
            assert!(!slice_ptr.is_null());
            let slice = unsafe { pyo3::Bound::from_owned_ptr(py, slice_ptr.cast()) };
            let slice_bound = slice.cast::<PySlice>().unwrap();
            let indices = slice_bound.indices(10).unwrap();
            assert_eq!((indices.start, indices.stop, indices.step), (2, 8, 2));
        });
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

    #[test]
    fn slice_unpack_and_adjust() {
        Python::attach(|py| {
            let slice = PySlice::new(py, 1, 9, 2);
            let mut start = 0;
            let mut stop = 0;
            let mut step = 0;
            let ret = unsafe {
                super::PySlice_Unpack(slice.as_ptr().cast(), &mut start, &mut stop, &mut step)
            };
            assert_eq!(ret, 0);
            assert_eq!((start, stop, step), (1, 9, 2));

            let len = unsafe { super::PySlice_AdjustIndices(5, &mut start, &mut stop, step) };
            assert_eq!(len, 2);
            assert_eq!((start, stop), (1, 5));
        });
    }

    #[test]
    fn slice_get_indices_ex() {
        Python::attach(|py| {
            let slice = PySlice::new(py, 1, 5, 2);
            let mut start = 0;
            let mut stop = 0;
            let mut step = 0;
            let mut slicelen = 0;
            let ret = unsafe {
                super::PySlice_GetIndicesEx(
                    slice.as_ptr().cast(),
                    10,
                    &mut start,
                    &mut stop,
                    &mut step,
                    &mut slicelen,
                )
            };
            assert_eq!(ret, 0);
            assert_eq!((start, stop, step, slicelen), (1, 5, 2, 2));

            // Test clipping out of bounds
            let slice_out = PySlice::new(py, 1, 20, 1);
            let ret_out = unsafe {
                super::PySlice_GetIndicesEx(
                    slice_out.as_ptr().cast(),
                    5,
                    &mut start,
                    &mut stop,
                    &mut step,
                    &mut slicelen,
                )
            };
            assert_eq!(ret_out, 0);
            assert_eq!((start, stop, step, slicelen), (1, 5, 1, 4));

            // Test negative step with full slice
            let slice_rev = PySlice::new(py, 4, 0, -1);
            let ret_rev = unsafe {
                super::PySlice_GetIndicesEx(
                    slice_rev.as_ptr().cast(),
                    5,
                    &mut start,
                    &mut stop,
                    &mut step,
                    &mut slicelen,
                )
            };
            assert_eq!(ret_rev, 0);
            assert_eq!((start, stop, step, slicelen), (4, 0, -1, 4));
        });
    }

    #[test]
    fn slice_get_indices_bounds_and_negative_step() {
        Python::attach(|py| {
            let slice_ok = PySlice::new(py, 1, 4, 1);
            let mut start = 0;
            let mut stop = 0;
            let mut step = 0;
            let ret = unsafe {
                super::PySlice_GetIndices(
                    slice_ok.as_ptr().cast(),
                    5,
                    &mut start,
                    &mut stop,
                    &mut step,
                )
            };
            assert_eq!(ret, 0);
            assert_eq!((start, stop, step), (1, 4, 1));

            // Negative step within bounds
            let slice_neg = PySlice::new(py, 4, 1, -1);
            let ret_neg = unsafe {
                super::PySlice_GetIndices(
                    slice_neg.as_ptr().cast(),
                    5,
                    &mut start,
                    &mut stop,
                    &mut step,
                )
            };
            assert_eq!(ret_neg, 0);
            assert_eq!((start, stop, step), (4, 1, -1));

            // Out-of-bounds start/stop returns -1
            let slice_out = PySlice::new(py, 1, 20, 1);
            let ret_out = unsafe {
                super::PySlice_GetIndices(
                    slice_out.as_ptr().cast(),
                    10,
                    &mut start,
                    &mut stop,
                    &mut step,
                )
            };
            assert_eq!(ret_out, -1);

            // start == length returns -1
            let slice_start_len = PySlice::new(py, 5, 5, 1);
            let ret_start_len = unsafe {
                super::PySlice_GetIndices(
                    slice_start_len.as_ptr().cast(),
                    5,
                    &mut start,
                    &mut stop,
                    &mut step,
                )
            };
            assert_eq!(ret_start_len, -1);

            // Zero step returns -1 without setting exception
            let slice_zero = PySlice::new(py, 1, 4, 0);
            let ret_zero = unsafe {
                super::PySlice_GetIndices(
                    slice_zero.as_ptr().cast(),
                    5,
                    &mut start,
                    &mut stop,
                    &mut step,
                )
            };
            assert_eq!(ret_zero, -1);
            assert!(crate::pyerrors::PyErr_Occurred().is_null());
        });
    }
}
