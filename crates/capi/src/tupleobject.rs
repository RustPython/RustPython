use crate::PyObject;
use crate::pystate::with_vm;
use crate::handles::{exported_object_handle, resolve_object_handle};
use rustpython_vm::AsObject;
use rustpython_vm::PyResult;
use rustpython_vm::builtins::PyTuple;
use std::os::raw::c_int;

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_New(len: isize) -> *mut PyObject {
    with_vm(|vm| {
        if len < 0 {
            return Err(vm.new_system_error("negative size passed to PyTuple_New"));
        }
        Ok(PyTuple::new_uninit_ref(len as usize, &vm.ctx))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_SetItem(
    tuple: *mut PyObject,
    pos: isize,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| -> PyResult<()> {
        let tuple_obj = unsafe { &*resolve_object_handle(tuple) }.try_downcast_ref::<PyTuple>(vm)?;
        let index = usize::try_from(pos)
            .ok()
            .filter(|&i| i < tuple_obj.len())
            .ok_or_else(|| vm.new_index_error("tuple assignment index out of range"))?;
        let value = unsafe { (&*resolve_object_handle(value)).to_owned() };
        unsafe { PyTuple::set_item_unchecked(tuple_obj, index, value) };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_Size(tuple: *mut PyObject) -> isize {
    with_vm(|vm| {
        let tuple = unsafe { &*resolve_object_handle(tuple) }.try_downcast_ref::<PyTuple>(vm)?;
        Ok(tuple.__len__())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_GetItem(tuple: *mut PyObject, pos: isize) -> *mut PyObject {
    with_vm(|vm| {
        let tuple = unsafe { &*resolve_object_handle(tuple) }.try_downcast_ref::<PyTuple>(vm)?;
        let result: &PyObject = pos
            .try_into()
            .ok()
            .and_then(|index: usize| tuple.get(index))
            .ok_or_else(|| vm.new_index_error("tuple index out of range"))?;

        //Return borrowed reference
        Ok(unsafe { exported_object_handle(result.as_raw().cast_mut()) })
    })
}

#[cfg(test)]
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
}
