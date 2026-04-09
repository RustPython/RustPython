use crate::PyObject;
use crate::pystate::with_vm;
use rustpython_vm::builtins::PyTuple;

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_Size(_tuple: *mut PyObject) -> isize {
    with_vm(|vm| {
        let tuple = unsafe { &*_tuple }.try_downcast_ref::<PyTuple>(vm)?;
        Ok(tuple.__len__() as isize)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_GetItem(_tuple: *mut PyObject, pos: isize) -> *mut PyObject {
    with_vm(|vm| {
        let tuple = unsafe { &*_tuple }.try_downcast_ref::<PyTuple>(vm)?;
        let result: &PyObject = pos
            .try_into()
            .ok()
            .and_then(|index: usize| tuple.get(index))
            .ok_or_else(|| vm.new_index_error("tuple index out of range"))?;

        //Return borrowed reference
        Ok(result.as_raw())
    })
}
