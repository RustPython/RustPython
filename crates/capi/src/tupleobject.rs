use crate::PyObject;
use crate::pystate::with_vm;
use rustpython_vm::PyResult;
use rustpython_vm::builtins::PyTuple;

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_New(len: isize) -> *mut PyObject {
    with_vm(|vm| {
        if len == 0 {
            return Ok(vm.ctx.empty_tuple.to_owned());
        }

        Err(vm.new_not_implemented_error("PyTuple_New is not yet implemented"))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_SetItem(
    _tuple: *mut PyObject,
    _pos: isize,
    _value: *mut PyObject,
) -> *mut PyObject {
    with_vm::<PyResult, _>(|vm| {
        Err(vm.new_not_implemented_error("PyTuple_SetItem is not yet implemented"))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_Size(tuple: *mut PyObject) -> isize {
    with_vm(|vm| {
        let tuple = unsafe { &*tuple }.try_downcast_ref::<PyTuple>(vm)?;
        Ok(tuple.__len__())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_GetItem(tuple: *mut PyObject, pos: isize) -> *mut PyObject {
    with_vm(|vm| {
        let tuple = unsafe { &*tuple }.try_downcast_ref::<PyTuple>(vm)?;
        let result: &PyObject = pos
            .try_into()
            .ok()
            .and_then(|index: usize| tuple.get(index))
            .ok_or_else(|| vm.new_index_error("tuple index out of range"))?;

        //Return borrowed reference
        Ok(result.as_raw())
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
