use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use rustpython_vm::PyPayload;
use rustpython_vm::builtins::PyGenericAlias;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GenericAlias(
    origin: *mut PyObject,
    args: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let origin = unsafe { origin.assume_borrowed() }.to_owned();
        let args = unsafe { args.assume_borrowed() }.to_owned();
        PyGenericAlias::from_args(origin, args, vm).into_pyobject(vm)
    })
}
