use crate::with_vm;
use alloc::slice;
use core::ffi::c_int;
use rustpython_vm::builtins::{PyDict, PyStr, PyTuple};
use rustpython_vm::function::{FuncArgs, KwArgs};
use rustpython_vm::{AsObject, PyObject, PyObjectRef};

const PY_VECTORCALL_ARGUMENTS_OFFSET: usize = 1usize << (usize::BITS as usize - 1);

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Call(
    callable: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let callable = unsafe { &*callable };
        let args = unsafe { &*args }
            .try_downcast_ref::<PyTuple>(vm)?
            .iter()
            .cloned()
            .collect::<Vec<PyObjectRef>>();

        let kwargs: KwArgs = unsafe { kwargs.as_ref() }
            .map(|kwargs| kwargs.try_downcast_ref::<PyDict>(vm))
            .transpose()?
            .map_or_else(
                || KwArgs::default(),
                |kwargs| {
                    kwargs
                        .items_vec()
                        .iter()
                        .map(|(key, value)| todo!())
                        .collect()
                },
            );

        callable.call_with_args(FuncArgs::new(args, kwargs), vm)
    })
}

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
pub extern "C" fn PyObject_Vectorcall(
    callable: *mut PyObject,
    args: *const *mut PyObject,
    nargsf: usize,
    kwnames: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let args_len = nargsf & !PY_VECTORCALL_ARGUMENTS_OFFSET;
        let num_positional_args = args_len;

        let args = unsafe { slice::from_raw_parts(args, args_len) }
            .iter()
            .map(|arg| unsafe { &**arg }.to_owned())
            .collect::<Vec<_>>();

        let kwnames: Option<&[PyObjectRef]> = unsafe {
            kwnames
                .as_ref()
                .map(|tuple| Ok(&***tuple.try_downcast_ref::<PyTuple>(vm)?))
                .transpose()?
        };

        let callable = unsafe { &*callable };
        callable.vectorcall(args, num_positional_args, kwnames, vm)
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

        let (receiver, args) = unsafe { slice::from_raw_parts(args, args_len) }
            .split_first()
            .expect("PyObject_VectorcallMethod should always have at least one argument");

        let method_name = unsafe { (&*name).try_downcast_ref::<PyStr>(vm)? };
        let callable = unsafe { (&**receiver).get_attr(method_name, vm)? };

        Ok(PyObject_Vectorcall(
            callable.as_object().as_raw().cast_mut(),
            args.as_ptr(),
            nargsf - 1,
            kwnames,
        ))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GetItem(obj: *mut PyObject, key: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let key = unsafe { &*key };
        obj.get_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_SetItem(
    obj: *mut PyObject,
    key: *mut PyObject,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let key = unsafe { &*key };
        let value = unsafe { &*value }.to_owned();
        obj.set_item(key, value, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_DelItem(obj: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let key = unsafe { &*key };
        obj.del_item(key, vm)
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

#[unsafe(no_mangle)]
pub extern "C" fn PyNumber_Index(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*obj }.try_index(vm))
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyString};

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

    #[test]
    fn test_object_set_get_del_item() {
        Python::attach(|py| {
            let obj = PyDict::new(py).into_any();
            obj.set_item("key", "value").unwrap();
            assert_eq!(
                obj.get_item("key")
                    .unwrap()
                    .cast_into::<PyString>()
                    .unwrap(),
                "value"
            );
            obj.del_item("key").unwrap();
            assert!(obj.get_item("key").is_err());
        })
    }
}
