use crate::object::define_py_check;
use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use rustpython_vm::PyPayload;
use rustpython_vm::TryFromObject;
use rustpython_vm::builtins::{PyCallableIterator, PySequenceIterator};
use rustpython_vm::function::ArgCallable;

define_py_check!(exact fn PySeqIter_Check, types.iter_type);
define_py_check!(exact fn PyCallIter_Check, types.callable_iterator);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySeqIter_New(seq: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let seq = unsafe { seq.assume_borrowed() }.to_owned();
        Ok(PySequenceIterator::new(seq, vm)?.into_pyobject(vm))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCallIter_New(
    callable: *mut PyObject,
    sentinel: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let callable =
            ArgCallable::try_from_object(vm, unsafe { callable.assume_borrowed() }.to_owned())?;
        let sentinel = unsafe { sentinel.assume_borrowed() }.to_owned();
        Ok(PyCallableIterator::new(callable, sentinel).into_pyobject(vm))
    })
}
