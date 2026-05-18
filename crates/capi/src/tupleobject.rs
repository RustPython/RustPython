use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::c_int;
use core::slice;
use rustpython_vm::PyResult;
use rustpython_vm::builtins::PyTuple;
use rustpython_vm::sliceable::SliceableSequenceOp;

define_py_check!(fn PyTuple_Check, types.tuple_type);
define_py_check!(exact fn PyTuple_CheckExact, types.tuple_type);

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_New(len: isize) -> *mut PyObject {
    with_vm(|vm| {
        if len == 0 {
            return Ok(vm.ctx.empty_tuple.to_owned());
        }

        Err(vm.new_not_implemented_error(
            "PyTuple_New for non zero sized tuples is not supported, use PyTuple_FromArray instead",
        ))
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
            .map(|ptr| unsafe { &**ptr }.to_owned())
            .collect::<Vec<_>>();
        Ok(vm.new_tuple(elements))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_SetItem(
    _tuple: *mut PyObject,
    _pos: isize,
    _value: *mut PyObject,
) -> c_int {
    with_vm::<PyResult<()>, _>(
        |vm| Err(vm.new_not_implemented_error("Tuple objects are immutable")),
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_Size(tuple: *mut PyObject) -> isize {
    with_vm(|vm| {
        let tuple = unsafe { &*tuple }.try_downcast_ref::<PyTuple>(vm)?;
        Ok(tuple.__len__())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTuple_GetItem(tuple: *mut PyObject, pos: isize) -> *mut PyObject {
    with_vm(|vm| {
        let tuple = unsafe { &*tuple }.try_downcast_ref::<PyTuple>(vm)?;
        let result: &PyObject = pos
            .try_into()
            .ok()
            .and_then(|index: usize| tuple.get(index))
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
        let tuple = unsafe { &*tuple }.try_downcast_ref::<PyTuple>(vm)?;
        let len = tuple.__len__() as isize;
        let low = low.clamp(0, len);
        let high = high.clamp(low, len);
        let slice = tuple.do_slice(low as usize..high as usize);
        Ok(vm.ctx.new_tuple(slice))
    })
}

#[cfg(false)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyTuple;

    #[test]
    fn test_empty_tuple() {
        Python::attach(|py| {
            let tuple = PyTuple::empty(py);
            assert_eq!(tuple.len(), 0);
        })
    }

    #[test]
    fn test_tuple_into_python() {
        Python::attach(|py| {
            let tuple = (1, 2, 3).into_pyobject(py).unwrap();
            assert_eq!(tuple.len(), 3);
        })
    }

    #[test]
    fn test_tuple_get_slice() {
        Python::attach(|py| {
            let tuple = (1, 2, 3).into_pyobject(py).unwrap();
            let slice = tuple.get_slice(1, 2);
            assert_eq!(slice.extract::<(u32,)>().unwrap(), (2,));
        })
    }
}
