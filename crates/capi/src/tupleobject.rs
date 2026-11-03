use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::util::FfiPtrExt;
use core::ffi::c_int;
use core::ptr;
use core::slice;
use rustpython_vm::AsObject;
use rustpython_vm::builtins::PyTuple;
use rustpython_vm::sliceable::SliceableSequenceOp;

define_py_check!(fn PyTuple_Check, types.tuple_type);
define_py_check!(exact fn PyTuple_CheckExact, types.tuple_type);

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_New(len: isize) -> *mut PyObject {
    with_vm(|vm| {
        if len == 0 {
            Ok(vm.ctx.empty_tuple.to_owned())
        } else {
            let len: usize = len
                .try_into()
                .map_err(|_| vm.new_system_error("Negative size passed to PyTuple_New"))?;
            Ok(vm.new_tuple(vec![vm.ctx.none(); len]))
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_FromArray(
    array: *const *mut PyObject,
    size: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let size = size
            .try_into()
            .map_err(|_| vm.new_system_error("negative size passed to Tuple_FromArray"))?;
        let slice = unsafe { slice::from_raw_parts(array, size) };
        let elements = slice
            .iter()
            .map(|ptr| unsafe { ptr.assume_borrowed() }.to_owned())
            .collect::<Vec<_>>();
        Ok(vm.new_tuple(elements))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_Pack(len: isize, mut args: ...) -> *mut PyObject {
    with_vm(|vm| {
        let len = len
            .try_into()
            .map_err(|_| vm.new_system_error("negative size passed to Tuple_Pack"))?;
        let items = core::iter::repeat_with(|| unsafe {
            args.next_arg::<*mut PyObject>()
                .assume_borrowed()
                .to_owned()
        })
        .take(len)
        .collect::<Vec<_>>();

        Ok(vm.new_tuple(items))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_SetItem(
    tuple: *mut PyObject,
    pos: isize,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let tuple = unsafe { tuple.assume_borrowed_and_cast::<PyTuple>(vm)? };
        let value = unsafe { value.assume_owned() };
        let index: usize = pos
            .try_into()
            .map_err(|_| vm.new_index_error("negative position"))?;

        if tuple.len() <= index {
            Err(vm.new_index_error("tuple index out of range"))
        } else if tuple.as_object().strong_count() > 1 {
            Err(vm.new_system_error("Tuple objects are immutable when the reference count > 1"))
        } else {
            // SAFETY: We have exclusive access to the tuple payload (strong_count == 1),
            // so mutating the selected element in place is valid.
            let data_ptr = unsafe { tuple.as_ptr().add(index).cast_mut() };
            unsafe { ptr::replace(data_ptr, value) };
            Ok(())
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_Size(tuple: *mut PyObject) -> isize {
    with_vm(|vm| {
        let tuple = unsafe { tuple.assume_borrowed_and_cast::<PyTuple>(vm) }?;
        Ok(tuple.as_slice().len())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_GetItem(tuple: *mut PyObject, pos: isize) -> *mut PyObject {
    with_vm(|vm| {
        let tuple = unsafe { tuple.assume_borrowed_and_cast::<PyTuple>(vm) }?;
        let result: &PyObject = pos
            .try_into()
            .ok()
            .and_then(|index: usize| tuple.as_slice().get(index))
            .ok_or_else(|| vm.new_index_error("tuple index out of range"))?;

        Ok(result.as_raw())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_GetSlice(
    tuple: *mut PyObject,
    low: isize,
    high: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let tuple = unsafe { tuple.assume_borrowed_and_cast::<PyTuple>(vm) }?;
        let len = tuple.as_slice().len() as isize;
        let low = low.clamp(0, len);
        let high = high.clamp(low, len);
        let slice = tuple.as_slice().do_slice(low as usize..high as usize);
        Ok(vm.ctx.new_tuple(slice))
    })
}

#[cfg(test)]
mod tests {
    use pyo3::ffi;
    use pyo3::prelude::*;
    use pyo3::types::PyTuple;

    #[test]
    fn empty_tuple() {
        Python::attach(|py| {
            let tuple = PyTuple::empty(py);
            assert_eq!(tuple.len(), 0);
        })
    }

    #[test]
    fn tuple_into_python() {
        Python::attach(|py| {
            let tuple = (1, 2, 3).into_pyobject(py).unwrap();
            assert_eq!(tuple.len(), 3);
        })
    }

    #[test]
    fn tuple_get_slice() {
        Python::attach(|py| {
            let tuple = (1, 2, 3).into_pyobject(py).unwrap();
            let slice = tuple.get_slice(1, 2);
            assert_eq!(slice.extract::<(u32,)>().unwrap(), (2,));
        })
    }

    #[test]
    fn tuple_is_mutable_with_one_ref() {
        Python::attach(|py| {
            let value: Bound<'_, _> = 1.into_pyobject(py).unwrap();
            let tuple: Bound<'_, PyTuple> =
                unsafe { Bound::from_owned_ptr(py, ffi::PyTuple_New(1)).cast_into_unchecked() };
            assert!(tuple.get_item(0).unwrap().is_none());
            unsafe {
                assert_eq!(
                    ffi::PyTuple_SetItem(tuple.as_ptr(), 0, value.clone().into_ptr()),
                    0
                );
            }
            assert_eq!(tuple.get_item(0).unwrap().extract::<u32>().unwrap(), 1);
            let _new_ref = tuple.clone();
            unsafe {
                assert_eq!(
                    ffi::PyTuple_SetItem(tuple.as_ptr(), 0, value.clone().into_ptr()),
                    -1
                );
            }
            assert!(PyErr::take(py).is_some())
        })
    }
}
