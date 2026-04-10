use crate::with_vm;
use alloc::slice;
use core::ffi::c_int;
use rustpython_vm::builtins::{PyDict, PyStr, PyTuple};
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
#[cfg(feature = "nightly")]
pub unsafe extern "C" fn PyObject_CallMethodObjArgs(
    receiver: *mut PyObject,
    name: *mut PyObject,
    mut args: ...
) -> *mut PyObject {
    with_vm(|vm| {
        let mut arguments: Vec<PyObjectRef> = vec![];
        loop {
            if let Some(arg) = core::ptr::NonNull::new(unsafe { args.arg::<*mut PyObject>() }) {
                arguments.push(unsafe { arg.as_ref() }.to_owned());
            } else {
                break;
            }
        }

        let method_name = unsafe { (&*name).try_downcast_ref::<PyStr>(vm)? };
        let callable = unsafe { (&*receiver).get_attr(method_name, vm)? };
        callable.call(arguments, vm)
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

#[unsafe(no_mangle)]
pub extern "C" fn PySequence_Contains(obj: *mut PyObject, value: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let value = unsafe { &mut *value };
        match obj.try_sequence(vm) {
            Ok(sequence) => sequence.contains(value, vm),
            Err(type_err) => {
                // TODO Dict should implement sequence protocol, but for now we can special case it
                if let Some(dict) = obj.downcast_ref::<PyDict>() {
                    Ok(dict.contains_key(value, vm))
                } else {
                    Err(type_err)
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyString;

    #[test]
    #[cfg(feature = "nightly")]
    fn test_call_method0() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert_eq!(
                string.call_method0("upper").unwrap().str().unwrap(),
                "HELLO, WORLD!"
            );
        })
    }

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
