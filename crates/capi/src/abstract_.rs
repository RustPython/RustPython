use crate::pystate::with_vm;
use rustpython_vm::PyObject;
use rustpython_vm::builtins::{PyStr, PyTuple};
use std::slice;

const PY_VECTORCALL_ARGUMENTS_OFFSET: usize = 1usize << (usize::BITS as usize - 1);

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_CallNoArgs(callable: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        if callable.is_null() {
            vm.push_exception(Some(vm.new_system_error(
                "PyObject_CallNoArgs called with null callable".to_owned(),
            )));
            return std::ptr::null_mut();
        }

        let callable = unsafe { &*callable };
        callable.call((), vm).map_or_else(
            |err| {
                vm.push_exception(Some(err));
                std::ptr::null_mut()
            },
            |result| result.into_raw().as_ptr(),
        )
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_VectorcallMethod(
    name: *mut PyObject,
    args: *const *mut PyObject,
    nargsf: usize,
    kwnames: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let args_len = nargsf & !PY_VECTORCALL_ARGUMENTS_OFFSET;
        let num_positional_args = args_len - 1;

        let (receiver, args) = unsafe { slice::from_raw_parts(args, args_len) }
            .split_first()
            .expect("PyObject_VectorcallMethod should always have at least one argument");

        let method_name = unsafe { (&*name).downcast_unchecked_ref::<PyStr>() };
        let callable = match unsafe {
            (&**receiver)
                .get_attr(method_name, vm)
        } {
            Ok(obj) => obj,
            Err(err) => {
                vm.push_exception(Some(err));
                return std::ptr::null_mut()
            },
        };

        let args = args
            .iter()
            .map(|arg| unsafe { &**arg }.to_owned())
            .collect::<Vec<_>>();

        let kwnames = unsafe {
            kwnames
                .as_ref()
                .map(|tuple| &***tuple.downcast_unchecked_ref::<PyTuple>())
        };

        callable
            .vectorcall(args, num_positional_args, kwnames, vm)
            .map_or_else(
                |err| {
                    vm.push_exception(Some(err));
                    std::ptr::null_mut()
                },
                |obj| obj.into_raw().as_ptr(),
            )
    })
}
