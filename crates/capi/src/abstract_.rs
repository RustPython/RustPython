use crate::handles::{exported_object_handle, exported_object_wrapper, resolve_object_handle};
use crate::with_vm;
use alloc::slice;
use core::ffi::c_int;
use rustpython_vm::builtins::{PyDict, PyStr, PyTuple};
use rustpython_vm::protocol::{PyIter, PyIterReturn};
use rustpython_vm::types::PyComparisonOp;
use rustpython_vm::{AsObject, PyObject, PyObjectRef};

const PY_VECTORCALL_ARGUMENTS_OFFSET: usize = 1usize << (usize::BITS as usize - 1);

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_CallNoArgs(callable: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        if callable.is_null() {
            return Err(
                vm.new_system_error("PyObject_CallNoArgs called with null callable".to_owned())
            );
        }

        let callable = unsafe { &*resolve_object_handle(callable) };
        callable.call((), vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Call(
    callable: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let callable = unsafe { &*resolve_object_handle(callable) };
        let mut func_args = rustpython_vm::function::FuncArgs::default();

        if !args.is_null() {
            let tuple = unsafe { &*resolve_object_handle(args) }.try_downcast_ref::<PyTuple>(vm)?;
            func_args.args.extend(tuple.iter().map(|arg| arg.to_owned()));
        }

        if !kwargs.is_null() {
            let kwargs = unsafe { &*resolve_object_handle(kwargs) }.try_downcast_ref::<PyDict>(vm)?;
            for (key, value) in kwargs.items_vec() {
                let key = key.try_downcast::<PyStr>(vm)?;
                func_args
                    .kwargs
                    .insert(key.to_string_lossy().into_owned(), value);
            }
        }

        callable.call(func_args, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_CallObject(callable: *mut PyObject, args: *mut PyObject) -> *mut PyObject {
    PyObject_Call(callable, args, core::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GetIter(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        let iter = obj.get_iter(vm)?;
        Ok(unsafe {
            exported_object_wrapper(
                iter.as_object().as_raw().cast_mut(),
                core::mem::size_of::<usize>() * 2,
            )
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn RustPython_PyObject_CallMethodObjArgsArray(
    receiver: *mut PyObject,
    name: *mut PyObject,
    args: *const *mut PyObject,
    nargs: usize,
) -> *mut PyObject {
    with_vm(|vm| {
        let arguments = if args.is_null() || nargs == 0 {
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(args, nargs) }
                .iter()
                .map(|arg| unsafe { &*resolve_object_handle(*arg) }.to_owned())
                .collect::<Vec<PyObjectRef>>()
        };

        let method_name = unsafe { (&*resolve_object_handle(name)).try_downcast_ref::<PyStr>(vm)? };
        let callable = unsafe { (&*resolve_object_handle(receiver)).get_attr(method_name, vm)? };
        callable.call(arguments, vm)
    })
}

fn compare_op_from_c_int(opid: c_int) -> Option<PyComparisonOp> {
    Some(match opid {
        0 => PyComparisonOp::Lt,
        1 => PyComparisonOp::Le,
        2 => PyComparisonOp::Eq,
        3 => PyComparisonOp::Ne,
        4 => PyComparisonOp::Gt,
        5 => PyComparisonOp::Ge,
        _ => return None,
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

        let kwnames: Option<&[PyObjectRef]> = unsafe {
            kwnames
                .as_ref()
                .map(|tuple| {
                    let tuple =
                        (&*resolve_object_handle(tuple as *const _ as *mut _)).try_downcast_ref::<PyTuple>(vm)?;
                    Ok(&***tuple)
                })
                .transpose()?
        };

        let kw_count = kwnames.map_or(0, |tuple| tuple.len());
        let args = unsafe { slice::from_raw_parts(args, args_len + kw_count) }
            .iter()
            .map(|arg| unsafe { &*resolve_object_handle(*arg) }.to_owned())
            .collect::<Vec<_>>();

        let callable = unsafe { &*resolve_object_handle(callable) };
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

        let method_name = unsafe { (&*resolve_object_handle(name)).try_downcast_ref::<PyStr>(vm)? };
        let callable = unsafe { (&*resolve_object_handle(*receiver)).get_attr(method_name, vm)? };

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
        let obj = unsafe { &*resolve_object_handle(obj) };
        let key = unsafe { &*resolve_object_handle(key) };
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
        let obj = unsafe { &*resolve_object_handle(obj) };
        let key = unsafe { &*resolve_object_handle(key) };
        let value = unsafe { &*resolve_object_handle(value) }.to_owned();
        obj.set_item(key, value, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_DelItem(obj: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        let key = unsafe { &*resolve_object_handle(key) };
        obj.del_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Hash(obj: *mut PyObject) -> i64 {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        obj.hash(vm).map(|hash| hash as i64)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_IsInstance(obj: *mut PyObject, cls: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        let cls = unsafe { &*resolve_object_handle(cls) };
        obj.is_instance(cls, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_RichCompare(
    left: *mut PyObject,
    right: *mut PyObject,
    opid: c_int,
) -> *mut PyObject {
    with_vm(|vm| {
        let Some(op) = compare_op_from_c_int(opid) else {
            return Err(vm.new_system_error(format!(
                "PyObject_RichCompare called with invalid opid {opid}"
            )));
        };
        let left = unsafe { &*resolve_object_handle(left) }.to_owned();
        let right = unsafe { &*resolve_object_handle(right) }.to_owned();
        left.rich_compare(right, op, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Size(obj: *mut PyObject) -> isize {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        obj.length(vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyMapping_Items(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        let items = vm.call_method(obj, "items", ())?;
        let iter = items.get_iter(vm).map_err(|_| {
            vm.new_type_error(format!(
                "{}.items() returned a non-iterable (type {})",
                obj.class(),
                items.class()
            ))
        })?;
        Ok(vm.ctx.new_list(iter.try_to_value(vm)?))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyTuple_GetSlice(
    tuple: *mut PyObject,
    low: isize,
    high: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let tuple = unsafe { &*resolve_object_handle(tuple) }.try_downcast_ref::<PyTuple>(vm)?;
        let len = tuple.len() as isize;
        let start = low.clamp(0, len) as usize;
        let end = high.clamp(start as isize, len) as usize;
        let slice = tuple.as_slice()[start..end].to_vec();
        Ok(vm.ctx.new_tuple(slice))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PySequence_Contains(obj: *mut PyObject, value: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        let value = unsafe { &mut *resolve_object_handle(value) };
        if let Some(dict) = obj.downcast_ref::<PyDict>() {
            return Ok(dict.contains_key(value, vm));
        }
        obj.sequence_unchecked().contains(value, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PySequence_Check(obj: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        Ok(obj.try_sequence(vm).is_ok())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyIter_NextItem(iter: *mut PyObject, item: *mut *mut PyObject) -> c_int {
    let mut result = 0;
    let status: c_int = with_vm(|vm| -> rustpython_vm::PyResult<()> {
        let iter = unsafe { &*resolve_object_handle(iter) };
        let iter = PyIter::new(iter);
        match iter.next(vm)? {
            PyIterReturn::Return(obj) => {
                unsafe {
                    *item = exported_object_handle(obj.into_raw().as_ptr());
                }
                result = 1;
            }
            PyIterReturn::StopIteration(_) => {
                unsafe {
                    *item = core::ptr::null_mut();
                }
                result = 0;
            }
        }
        Ok(())
    });
    if status == -1 { -1 } else { result }
}

#[unsafe(no_mangle)]
pub extern "C" fn PyNumber_Index(o: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(o) };
        obj.try_index(vm)
            .map(|obj| unsafe { exported_object_handle(obj.as_object().as_raw().cast_mut()) })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GC_Track(_obj: *mut core::ffi::c_void) {}

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
