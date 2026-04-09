use crate::with_vm;
use alloc::slice;
use rustpython_vm::builtins::{PyStr, PyTuple};
use rustpython_vm::{PyObject, PyObjectRef, PyResult};

const PY_VECTORCALL_ARGUMENTS_OFFSET: usize = 1usize << (usize::BITS as usize - 1);

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_CallNoArgs(callable: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        if callable.is_null() {
            return Err(
                vm.new_system_error("PyObject_CallNoArgs called with null callable".to_owned())
            );
        }

        let callable = unsafe { &*callable };
        callable.call((), vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_VectorcallMethod(
    name: *mut PyObject,
    args: *const *mut PyObject,
    nargsf: usize,
    kwnames: *mut PyObject,
) -> *mut PyObject {
    with_vm::<PyResult>(|vm| {
        let args_len = nargsf & !PY_VECTORCALL_ARGUMENTS_OFFSET;
        let num_positional_args = args_len - 1;

        let (receiver, args) = unsafe { slice::from_raw_parts(args, args_len) }
            .split_first()
            .expect("PyObject_VectorcallMethod should always have at least one argument");

        let method_name = unsafe { (&*name).try_downcast_ref::<PyStr>(vm)? };
        let callable = unsafe { (&**receiver).get_attr(method_name, vm)? };

        let args = args
            .iter()
            .map(|arg| unsafe { &**arg }.to_owned())
            .collect::<Vec<_>>();

        let kwnames: Option<&[PyObjectRef]> = unsafe {
            kwnames
                .as_ref()
                .map(|tuple| Ok(&***tuple.try_downcast_ref::<PyTuple>(vm)?))
                .transpose()?
        };

        callable.vectorcall(args, num_positional_args, kwnames, vm)
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyString;

    #[test]
    fn test_call_method1() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert!(
                string
                    .call_method1("endswith", ("!",))
                    .unwrap()
                    .is_truthy()
                    .unwrap()
            );
        })
    }
}
