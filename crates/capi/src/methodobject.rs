use crate::PyObject;
use crate::object::PyTypeObject;
use crate::pystate::with_vm;
use bitflags::bitflags_match;
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use rustpython_vm::function::{FuncArgs, PyMethodFlags};
use rustpython_vm::{AsObject, PyObjectRef, PyResult, VirtualMachine};

type PyCFunction = unsafe extern "C" fn(slf: *mut PyObject, args: *mut PyObject) -> *mut PyObject;
type PyCFunctionWithKeywords = unsafe extern "C" fn(
    slf: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject;

#[repr(C)]
#[derive(Copy, Clone)]
pub union PyMethodPointer {
    function: PyCFunction,
    function_with_keywords: PyCFunctionWithKeywords,
}

#[repr(C)]
pub struct PyMethodDef {
    ml_name: *const c_char,
    ml_meth: PyMethodPointer,
    ml_flags: c_int,
    ml_doc: *const c_char,
}

fn c_function_wrapper(
    vm: &VirtualMachine,
    slf: Option<&PyObjectRef>,
    mut args: FuncArgs,
    method: PyMethodPointer,
    flags: PyMethodFlags,
) -> PyResult {
    let slf_ptr = slf
        .map(|slf| slf.as_object().as_raw().cast_mut())
        .unwrap_or_default();

    let arg_tuple = vm.ctx.new_tuple(core::mem::take(&mut args.args));
    let arg_tuple_ptr = arg_tuple.as_object().as_raw().cast_mut();

    let ret_ptr = bitflags_match!(flags, {
        PyMethodFlags::NOARGS => {
            let f = unsafe { method.function };
            unsafe { Ok(f(slf_ptr, arg_tuple_ptr)) }
        },
        PyMethodFlags::VARARGS => {
            let f = unsafe { method.function };
            unsafe { Ok(f(slf_ptr, arg_tuple_ptr)) }
        },
        PyMethodFlags::VARARGS | PyMethodFlags::KEYWORDS => {
            let f = unsafe { method.function_with_keywords };
            let kwargs = vm.ctx.new_dict();
            for (k, v) in args.kwargs {
                kwargs.set_item(&*k, v, vm)?;
            }
            let kwargs_ptr = kwargs.as_object().as_raw().cast_mut();
            unsafe { Ok(f(slf_ptr, arg_tuple_ptr, kwargs_ptr)) }
        },
        _ => panic!("Unexpected flags value: {flags:?}"),
    })?;

    let ret_ptr = NonNull::new(ret_ptr).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("Native function returned NULL, but there was no exception set")
    })?;
    Ok(unsafe { PyObjectRef::from_raw(ret_ptr) })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyCMethod_New(
    ml: *mut PyMethodDef,
    slf: *mut PyObject,
    _module: *mut PyObject,
    _cls: *mut PyTypeObject,
) -> *mut PyObject {
    with_vm(|vm| -> PyResult {
        let ml = unsafe { &*ml };
        let name = unsafe { CStr::from_ptr(ml.ml_name) }
            .to_str()
            .expect("Method name was not valid UTF-8");

        let doc = unsafe { CStr::from_ptr(ml.ml_doc) }
            .to_str()
            .expect("Method doc was not valid UTF-8");

        let flags = PyMethodFlags::from_bits(ml.ml_flags as u32)
            .expect("PyMethodDef contains unknown flags");

        assert!(
            !flags.intersects(
                PyMethodFlags::METHOD
                    | PyMethodFlags::FASTCALL
                    | PyMethodFlags::CLASS
                    | PyMethodFlags::STATIC
            ),
            "These flags are not yet supported: {:?}",
            flags
        );

        let method = ml.ml_meth;
        let slf = NonNull::new(slf).map(|ptr| unsafe { ptr.as_ref().to_owned() });
        let callable = move |args: FuncArgs, vm: &VirtualMachine| {
            c_function_wrapper(vm, slf.as_ref(), args, method, flags)
        };

        let method = vm.ctx.new_method_def(name, callable, flags, Some(doc));
        Ok(method.build_function(vm).into())
    })
}

#[cfg(test)]
mod tests {
    use pyo3::exceptions::PyException;
    use pyo3::ffi::{PyLong_FromLong, PyObject};
    use pyo3::prelude::*;
    use pyo3::types::{PyCFunction, PyInt, PyString};

    #[test]
    fn test_closure_function() {
        Python::attach(|py| {
            let f = PyCFunction::new_closure(py, None, None, |_args, _kwargs| "Hello from Rust!")
                .unwrap();

            assert_eq!(
                f.call0().unwrap().cast::<PyString>().unwrap(),
                "Hello from Rust!"
            );
        })
    }

    #[test]
    fn test_function_no_args() {
        Python::attach(|py| {
            unsafe extern "C" fn c_fn(_self: *mut PyObject, _args: *mut PyObject) -> *mut PyObject {
                assert!(_self.is_null());
                unsafe { PyLong_FromLong(4200) }
            }

            let py_fn = PyCFunction::new(py, c_fn, c"py_fn", c"", None).unwrap();

            let result = py_fn
                .call0()
                .unwrap()
                .cast::<PyInt>()
                .unwrap()
                .extract::<u32>()
                .unwrap();
            assert_eq!(result, 4200);
        })
    }

    #[test]
    fn test_closure_function_error() {
        Python::attach(|py| {
            let f = PyCFunction::new_closure(py, None, None, |_args, _kwargs| {
                Err::<(), _>(PyException::new_err("Something went wrong"))
            })
            .unwrap();

            let err = f.call0().unwrap_err();
            assert_eq!(
                err.value(py).repr().unwrap(),
                "Exception('Something went wrong')"
            );
        })
    }
}
