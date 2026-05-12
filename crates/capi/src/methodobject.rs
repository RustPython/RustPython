use crate::PyObject;
use crate::object::PyTypeObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use rustpython_vm::function::{FuncArgs, HeapMethodDef, PyMethodFlags};
use rustpython_vm::{AsObject, PyObjectRef, PyRef, PyResult, VirtualMachine};

define_py_check!(PyCFunction_Check, types.builtin_function_or_method_type);
define_py_check!(exact PyCFunction_CheckExact, types.builtin_function_or_method_type);

#[repr(C)]
pub struct PyMethodDef {
    pub(crate) ml_name: *const c_char,
    pub(crate) ml_meth: PyMethodPointer,
    pub(crate) ml_flags: c_int,
    pub(crate) ml_doc: *const c_char,
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(non_snake_case)]
pub union PyMethodPointer {
    PyCFunction: unsafe extern "C" fn(slf: *mut PyObject, args: *mut PyObject) -> *mut PyObject,
    PyCFunctionWithKeywords: unsafe extern "C" fn(
        slf: *mut PyObject,
        args: *mut PyObject,
        kwargs: *mut PyObject,
    ) -> *mut PyObject,
    PyCFunctionFast: unsafe extern "C" fn(
        slf: *mut PyObject,
        args: *mut *mut PyObject,
        nargs: isize,
    ) -> *mut PyObject,
    PyCFunctionFastWithKeywords: unsafe extern "C" fn(
        slf: *mut PyObject,
        args: *const *mut PyObject,
        nargs: isize,
        kwnames: *mut PyObject,
    ) -> *mut PyObject,
}

pub(crate) fn build_method_def(vm: &VirtualMachine, ml: &PyMethodDef) -> PyRef<HeapMethodDef> {
    let name = unsafe { CStr::from_ptr(ml.ml_name) }
        .to_str()
        .expect("Method name was not valid UTF-8");

    let doc = NonNull::new(ml.ml_doc.cast_mut()).map(|doc| {
        unsafe { CStr::from_ptr(doc.as_ptr()) }
            .to_str()
            .expect("Method doc was not valid UTF-8")
    });

    let flags =
        PyMethodFlags::from_bits(ml.ml_flags as u32).expect("PyMethodDef contains unknown flags");

    let method = ml.ml_meth;

    if flags.contains(PyMethodFlags::METHOD) {
        panic!("METH_METHOD is not supported on abi3")
    }

    let call_flags = flags
        & (PyMethodFlags::VARARGS
            | PyMethodFlags::KEYWORDS
            | PyMethodFlags::NOARGS
            | PyMethodFlags::O
            | PyMethodFlags::FASTCALL);

    bitflags::bitflags_match!(call_flags, {
        PyMethodFlags::NOARGS => {
            let f = unsafe { method.PyCFunction };
            let callable = move |mut args: FuncArgs, vm: &VirtualMachine| {
                let slf = take_self_arg(&mut args, flags);
                let slf_ptr = slf
                    .as_ref()
                    .map(|obj| obj.as_object().as_raw().cast_mut())
                    .unwrap_or_default();
                let arg_tuple = vm.ctx.new_tuple(args.args);
                debug_assert!(arg_tuple.is_empty(), "Expected no arguments, but got some");
                let ret_ptr = unsafe { f(slf_ptr, arg_tuple.as_object().as_raw().cast_mut()) };
                ret_ptr_to_pyresult(vm, ret_ptr)
            };
            vm.ctx.new_method_def(name, callable, flags, doc)
        },
        PyMethodFlags::VARARGS => {
            let f = unsafe { method.PyCFunction };
            let callable = move |mut args: FuncArgs, vm: &VirtualMachine| {
                let slf = take_self_arg(&mut args, flags);
                let slf_ptr = slf
                    .as_ref()
                    .map(|obj| obj.as_object().as_raw().cast_mut())
                    .unwrap_or_default();
                let arg_tuple = vm.ctx.new_tuple(args.args);
                let ret_ptr = unsafe { f(slf_ptr, arg_tuple.as_object().as_raw().cast_mut()) };
                ret_ptr_to_pyresult(vm, ret_ptr)
            };
            vm.ctx.new_method_def(name, callable, flags, doc)
        },
        PyMethodFlags::VARARGS | PyMethodFlags::KEYWORDS => {
            let f = unsafe { method.PyCFunctionWithKeywords };
            let callable = move |mut args: FuncArgs, vm: &VirtualMachine| {
                let slf = take_self_arg(&mut args, flags);
                let slf_ptr = slf
                    .as_ref()
                    .map(|obj| obj.as_object().as_raw().cast_mut())
                    .unwrap_or_default();
                let arg_tuple = vm.ctx.new_tuple(args.args);
                let kwargs = vm.ctx.new_dict();
                for (k, v) in args.kwargs {
                    kwargs.set_item(&*k, v, vm)?;
                }
                let ret_ptr = unsafe {
                    f(
                        slf_ptr,
                        arg_tuple.as_object().as_raw().cast_mut(),
                        kwargs.as_object().as_raw().cast_mut(),
                    )
                };
                ret_ptr_to_pyresult(vm, ret_ptr)
            };
            vm.ctx.new_method_def(name, callable, flags, doc)
        },
        PyMethodFlags::FASTCALL | PyMethodFlags::KEYWORDS => {
            let f = unsafe { method.PyCFunctionFastWithKeywords };
            let callable = move |mut args: FuncArgs, vm: &VirtualMachine| {
                let slf = take_self_arg(&mut args, flags);
                let slf_ptr = slf
                    .as_ref()
                    .map(|obj| obj.as_object().as_raw().cast_mut())
                    .unwrap_or_default();
                let nargs = args.args.len();
                let mut fastcall_args = args.args;
                let mut kwnames_tuple = None;
                if !args.kwargs.is_empty() {
                    let mut kwnames = Vec::with_capacity(args.kwargs.len());
                    for (k, v) in args.kwargs {
                        kwnames.push(vm.ctx.new_str(k).into());
                        fastcall_args.push(v);
                    }
                    kwnames_tuple = Some(vm.ctx.new_tuple(kwnames));
                }
                let fastcall_arg_ptrs = fastcall_args
                    .iter()
                    .map(|obj| obj.as_object().as_raw().cast_mut())
                    .collect::<Vec<_>>();
                let kwnames_ptr = kwnames_tuple
                    .as_ref()
                    .map(|tuple| tuple.as_object().as_raw().cast_mut())
                    .unwrap_or(core::ptr::null_mut());
                let ret_ptr = unsafe {
                    f(
                        slf_ptr,
                        fastcall_arg_ptrs.as_ptr(),
                        nargs as isize,
                        kwnames_ptr,
                    )
                };
                ret_ptr_to_pyresult(vm, ret_ptr)
            };
            vm.ctx.new_method_def(name, callable, flags, doc)
        },
        PyMethodFlags::FASTCALL => {
            let callable = move |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult {
                todo!("METH_FASTCALL without METH_KEYWORDS is not supported yet")
            };
            vm.ctx.new_method_def(name, callable, flags, doc)
        },
        PyMethodFlags::O => {
            let callable = move |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult {
                todo!("METH_O is not supported yet")
            };
            vm.ctx.new_method_def(name, callable, flags, doc)
        },
        _ => {
            todo!("unsupported or invalid calling-convention flags")
        },
    })
}

fn ret_ptr_to_pyresult(vm: &VirtualMachine, ret_ptr: *mut PyObject) -> PyResult {
    let ret_ptr = NonNull::new(ret_ptr).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("Native function returned NULL, but there was no exception set")
    })?;
    Ok(unsafe { PyObjectRef::from_raw(ret_ptr) })
}

fn take_self_arg(args: &mut FuncArgs, flags: PyMethodFlags) -> Option<PyObjectRef> {
    if flags.contains(PyMethodFlags::STATIC) | args.args.is_empty() {
        None
    } else {
        Some(args.args.remove(0))
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn PyCMethod_New(
    ml: *mut PyMethodDef,
    slf: *mut PyObject,
    _module: *mut PyObject,
    _cls: *mut PyTypeObject,
) -> *mut PyObject {
    with_vm(|vm| -> PyResult {
        assert!(
            _cls.is_null(),
            "PyCMethod_New does not support METH_METHOD yet"
        );
        let ml = unsafe { &*ml };
        let zelf = unsafe { slf.as_ref().map(|obj| obj.to_owned()) };
        Ok(build_method_def(vm, ml).build_function(vm, zelf).into())
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
