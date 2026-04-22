use crate::PyObject;
use crate::object::PyTypeObject;
use crate::pystate::with_vm;
use crate::handles::exported_object_handle;
use crate::util::owned_from_exported_new_ref;
use bitflags::bitflags_match;
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use rustpython_vm::builtins::PyType;
use rustpython_vm::function::{FuncArgs, PyMethodFlags};
use rustpython_vm::{AsObject, Py, PyObjectRef, PyResult, VirtualMachine};

type PyCFunction = unsafe extern "C" fn(slf: *mut PyObject, args: *mut PyObject) -> *mut PyObject;
type PyCFunctionWithKeywords = unsafe extern "C" fn(
    slf: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject;
type PyCFunctionFast = unsafe extern "C" fn(
    slf: *mut PyObject,
    args: *const *mut PyObject,
    nargs: isize,
) -> *mut PyObject;
type PyCFunctionFastWithKeywords = unsafe extern "C" fn(
    slf: *mut PyObject,
    args: *const *mut PyObject,
    nargs: isize,
    kwnames: *mut PyObject,
) -> *mut PyObject;

#[repr(C)]
#[derive(Copy, Clone)]
pub union PyMethodPointer {
    function: PyCFunction,
    function_with_keywords: PyCFunctionWithKeywords,
    function_fast: PyCFunctionFast,
    function_fast_with_keywords: PyCFunctionFastWithKeywords,
}

#[repr(C)]
pub struct PyMethodDef {
    pub(crate) ml_name: *const c_char,
    pub(crate) ml_meth: PyMethodPointer,
    pub(crate) ml_flags: c_int,
    pub(crate) ml_doc: *const c_char,
}

fn c_function_wrapper(
    vm: &VirtualMachine,
    slf: Option<&PyObjectRef>,
    mut args: FuncArgs,
    method: PyMethodPointer,
    flags: PyMethodFlags,
) -> PyResult {
    let slf_ptr = slf
        .map(|slf| unsafe { exported_object_handle(slf.as_object().as_raw().cast_mut()) })
        .unwrap_or_default();

    let arg_tuple = vm.ctx.new_tuple(core::mem::take(&mut args.args));
    let arg_tuple_ptr = arg_tuple.as_object().as_raw().cast_mut();
    let call_flags = flags & !(PyMethodFlags::METHOD | PyMethodFlags::CLASS | PyMethodFlags::STATIC);

    let ret_ptr = bitflags_match!(call_flags, {
        PyMethodFlags::NOARGS => {
            let f = unsafe { method.function };
            unsafe { Ok(f(slf_ptr, arg_tuple_ptr)) }
        },
        PyMethodFlags::O => {
            let f = unsafe { method.function };
            let arg = arg_tuple
                .as_slice()
                .first()
                .map(|obj| unsafe { exported_object_handle(obj.as_object().as_raw().cast_mut()) })
                .unwrap_or(core::ptr::null_mut());
            unsafe { Ok(f(slf_ptr, arg)) }
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
        PyMethodFlags::FASTCALL => {
            let f = unsafe { method.function_fast };
            let exported_args: Vec<*mut PyObject> = arg_tuple
                .as_slice()
                .iter()
                .map(|obj| unsafe { exported_object_handle(obj.as_object().as_raw().cast_mut()) })
                .collect();
            unsafe { Ok(f(slf_ptr, exported_args.as_ptr(), exported_args.len() as isize)) }
        },
        PyMethodFlags::FASTCALL | PyMethodFlags::KEYWORDS => {
            let f = unsafe { method.function_fast_with_keywords };
            let mut exported_args: Vec<*mut PyObject> = arg_tuple
                .as_slice()
                .iter()
                .map(|obj| unsafe { exported_object_handle(obj.as_object().as_raw().cast_mut()) })
                .collect();
            let mut kwarg_values = Vec::with_capacity(args.kwargs.len());
            let mut kwnames = Vec::with_capacity(args.kwargs.len());
            for (k, v) in args.kwargs {
                kwnames.push(vm.ctx.new_str(k.to_string()));
                exported_args.push(unsafe { exported_object_handle(v.as_object().as_raw().cast_mut()) });
                kwarg_values.push(v);
            }
            let kwnames_tuple = vm.ctx.new_tuple(
                kwnames
                    .iter()
                    .map(|name| name.as_object().to_owned())
                    .collect(),
            );
            unsafe {
                Ok(f(
                    slf_ptr,
                    exported_args.as_ptr(),
                    arg_tuple.len() as isize,
                    exported_object_handle(kwnames_tuple.as_object().as_raw().cast_mut()),
                ))
            }
        },
        _ => panic!("Unexpected flags value: {flags:?}"),
    })?;

    let ret_ptr = NonNull::new(ret_ptr).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("Native function returned NULL, but there was no exception set")
    })?;
    Ok(unsafe { owned_from_exported_new_ref(ret_ptr.as_ptr()) })
}

pub(crate) fn build_tp_method(
    ml: &PyMethodDef,
    class: &'static Py<PyType>,
    vm: &VirtualMachine,
) -> (String, PyObjectRef) {
    let name = unsafe { CStr::from_ptr(ml.ml_name) }
        .to_str()
        .expect("Method name was not valid UTF-8")
        .to_owned();
    let doc = unsafe { CStr::from_ptr(ml.ml_doc) }
        .to_str()
        .expect("Method doc was not valid UTF-8")
        .to_owned();
    let raw_flags = PyMethodFlags::from_bits(ml.ml_flags as u32)
        .expect("PyMethodDef contains unknown flags");
    let effective_flags = if raw_flags.intersects(PyMethodFlags::CLASS | PyMethodFlags::STATIC) {
        raw_flags
    } else {
        raw_flags | PyMethodFlags::METHOD
    };
    let method = ml.ml_meth;
    let heap_def = vm.ctx.new_method_def(
        Box::leak(name.clone().into_boxed_str()),
        move |slf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine| {
            c_function_wrapper(vm, Some(&slf), args, method, effective_flags)
        },
        effective_flags,
        Some(Box::leak(doc.into_boxed_str())),
    );
    let descriptor = heap_def.build_proper_method(class, vm);
    (name, descriptor)
}

#[unsafe(no_mangle)]
pub extern "C" fn PyCMethod_New(
    ml: *mut PyMethodDef,
    slf: *mut PyObject,
    _module: *mut PyObject,
    cls: *mut PyTypeObject,
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

        let method = ml.ml_meth;
        let slf = NonNull::new(slf).map(|ptr| unsafe { ptr.as_ref().to_owned() });
        let slf_for_callable = slf.clone();
        let callable = move |args: FuncArgs, vm: &VirtualMachine| {
            c_function_wrapper(vm, slf_for_callable.as_ref(), args, method, flags)
        };

        let method = vm.ctx.new_method_def(name, callable, flags, Some(doc));
        if flags.contains(PyMethodFlags::METHOD) || flags.contains(PyMethodFlags::CLASS) {
            let class = if !cls.is_null() {
                unsafe { &*crate::handles::resolve_type_handle(cls) }
            } else {
                return Err(vm.new_system_error(format!(
                    "PyCMethod_New missing class for flagged method {name}"
                )));
            };
            Ok(method.build_proper_method(class, vm))
        } else if flags.contains(PyMethodFlags::STATIC) && !cls.is_null() {
            let class = unsafe { &*crate::handles::resolve_type_handle(cls) };
            Ok(method.build_proper_method(class, vm))
        } else {
            Ok(method.build_function(vm).into())
        }
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
