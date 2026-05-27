use crate::PyObject;
use crate::object::PyTypeObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use rustpython_vm::function::{FuncArgs, HeapMethodDef, PosArgs, PyMethodFlags};
use rustpython_vm::{AsObject, PyObjectRef, PyRef, PyResult, VirtualMachine};

define_py_check!(fn PyCFunction_Check, types.builtin_function_or_method_type);
define_py_check!(exact fn PyCFunction_CheckExact, types.builtin_function_or_method_type);

#[repr(C)]
pub struct PyMethodDef {
    pub ml_name: *const c_char,
    pub ml_meth: PyMethodPointer,
    pub ml_flags: c_int,
    pub ml_doc: *const c_char,
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(non_snake_case)]
pub union PyMethodPointer {
    pub PyCFunction: unsafe extern "C" fn(slf: *mut PyObject, args: *mut PyObject) -> *mut PyObject,
    pub PyCFunctionWithKeywords: unsafe extern "C" fn(
        slf: *mut PyObject,
        args: *mut PyObject,
        kwargs: *mut PyObject,
    ) -> *mut PyObject,
    pub PyCFunctionFast: unsafe extern "C" fn(
        slf: *mut PyObject,
        args: *const *mut PyObject,
        nargs: isize,
    ) -> *mut PyObject,
    pub PyCFunctionFastWithKeywords: unsafe extern "C" fn(
        slf: *mut PyObject,
        args: *const *mut PyObject,
        nargs: isize,
        kwnames: *mut PyObject,
    ) -> *mut PyObject,
}

pub(crate) fn build_method_def(
    vm: &VirtualMachine,
    ml: &PyMethodDef,
    has_self: bool,
) -> PyResult<PyRef<HeapMethodDef>> {
    let name = unsafe { CStr::from_ptr(ml.ml_name) }
        .to_str()
        .map_err(|_| vm.new_system_error("Method name was not valid UTF-8"))?;

    let doc = NonNull::new(ml.ml_doc.cast_mut())
        .map(|doc| {
            unsafe { CStr::from_ptr(doc.as_ptr()) }
                .to_str()
                .map_err(|_| vm.new_system_error("Method doc was not valid UTF-8"))
        })
        .transpose()?;

    let flags = PyMethodFlags::from_bits(ml.ml_flags as u32)
        .ok_or_else(|| vm.new_system_error("PyMethodDef contains unknown flags"))?;

    let method = ml.ml_meth;

    if flags.contains(PyMethodFlags::METHOD) {
        return Err(vm.new_system_error("METH_METHOD is not supported on abi3"));
    }

    let call_flags = flags
        & (PyMethodFlags::VARARGS
            | PyMethodFlags::KEYWORDS
            | PyMethodFlags::NOARGS
            | PyMethodFlags::O
            | PyMethodFlags::FASTCALL);

    bitflags::bitflags_match!(call_flags, {
        PyMethodFlags::NOARGS => {
            if has_self {
                let callable = move |zelf: PyObjectRef, vm: &VirtualMachine| unsafe {
                    let f = method.PyCFunction;
                    let ret_ptr = f(zelf.as_raw().cast_mut(), core::ptr::null_mut());
                    ret_ptr_to_pyresult(vm, ret_ptr)
                };
                Ok(vm.ctx.new_method_def(name, callable, flags, doc))
            } else {
                let callable = move |vm: &VirtualMachine| unsafe {
                    let f = method.PyCFunction;
                    let ret_ptr = f(core::ptr::null_mut(), core::ptr::null_mut());
                    ret_ptr_to_pyresult(vm, ret_ptr)
                };
                Ok(vm.ctx.new_method_def(name, callable, flags, doc))
            }
        },
        PyMethodFlags::VARARGS => {
            let callable = move |args: PosArgs, vm: &VirtualMachine| unsafe {
                call_function(vm, method, flags, Some(args))
            };
            Ok(vm.ctx.new_method_def(name, callable, flags, doc))
        },
        PyMethodFlags::VARARGS | PyMethodFlags::KEYWORDS => {
            let callable = move | args: FuncArgs, vm: &VirtualMachine| unsafe {
                call_function_with_keywords(vm, method, flags, args)
            };
            Ok(vm.ctx.new_method_def(name, callable, flags, doc))
        },
        PyMethodFlags::FASTCALL | PyMethodFlags::KEYWORDS => {
            let callable = move |args: FuncArgs, vm: &VirtualMachine| unsafe {
                call_fast_function_with_keywords(vm, method, flags, args)
            };
            Ok(vm.ctx.new_method_def(name, callable, flags, doc))
        },
        PyMethodFlags::FASTCALL => {
            let callable = move |args: PosArgs, vm: &VirtualMachine| unsafe {
                call_fast_function(vm, method, flags, args)
            };
            Ok(vm.ctx.new_method_def(name, callable, flags, doc))
        },
        PyMethodFlags::O => {
            let f = unsafe { method.PyCFunction };
            if has_self {
                let callable = move |zelf: PyObjectRef, arg: PyObjectRef, vm: &VirtualMachine| -> PyResult {
                    let ret_ptr = unsafe { f(zelf.as_raw().cast_mut(), arg.as_raw().cast_mut()) };
                    ret_ptr_to_pyresult(vm, ret_ptr)
                };
                Ok(vm.ctx.new_method_def(name, callable, flags, doc))
            } else {
                let callable = move |arg: PyObjectRef, vm: &VirtualMachine| -> PyResult {
                    let ret_ptr = unsafe { f(core::ptr::null_mut(), arg.as_raw().cast_mut()) };
                    ret_ptr_to_pyresult(vm, ret_ptr)
                };
                Ok(vm.ctx.new_method_def(name, callable, flags, doc))
            }
        },
        _ => {
            Err(vm.new_system_error(format!(
                "function {name} has unsupported or invalid calling-convention flags: {flags:?}"
            )))
        },
    })
}

unsafe fn call_function<A: Into<FuncArgs>>(
    vm: &VirtualMachine,
    method: PyMethodPointer,
    flags: PyMethodFlags,
    args: Option<A>,
) -> PyResult {
    let f = unsafe { method.PyCFunction };
    let (slf, arg_tuple) = if let Some(mut args) = args.map(Into::into) {
        let slf = take_self_arg(&mut args, flags);
        let arg_tuple = vm.ctx.new_tuple(args.args);
        (slf, Some(arg_tuple))
    } else {
        (None, None)
    };

    let slf_ptr = slf
        .as_ref()
        .map(|obj| obj.as_object().as_raw().cast_mut())
        .unwrap_or_default();

    let arg_ptr = arg_tuple
        .as_ref()
        .map(|tuple| tuple.as_object().as_raw().cast_mut())
        .unwrap_or_default();

    let ret_ptr = unsafe { f(slf_ptr, arg_ptr) };
    ret_ptr_to_pyresult(vm, ret_ptr)
}

unsafe fn call_function_with_keywords(
    vm: &VirtualMachine,
    method: PyMethodPointer,
    flags: PyMethodFlags,
    mut args: FuncArgs,
) -> PyResult {
    let f = unsafe { method.PyCFunctionWithKeywords };
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
}

unsafe fn call_fast_function_with_keywords(
    vm: &VirtualMachine,
    method: PyMethodPointer,
    flags: PyMethodFlags,
    mut args: FuncArgs,
) -> PyResult {
    let f = unsafe { method.PyCFunctionFastWithKeywords };
    let slf = take_self_arg(&mut args, flags);
    let slf_ptr = slf
        .as_ref()
        .map(|obj| obj.as_object().as_raw().cast_mut())
        .unwrap_or_default();
    let nargs = args.args.len();
    let mut fastcall_args = args.args;
    let kwnames_tuple = if !args.kwargs.is_empty() {
        let mut kwnames = Vec::with_capacity(args.kwargs.len());
        for (k, v) in args.kwargs {
            kwnames.push(vm.ctx.new_str(k).into());
            fastcall_args.push(v);
        }
        Some(vm.ctx.new_tuple(kwnames))
    } else {
        None
    };
    let kwnames_ptr = kwnames_tuple
        .as_ref()
        .map(|tuple| tuple.as_object().as_raw().cast_mut())
        .unwrap_or_default();
    // SAFETY: PyObjectRef is repr(transparent) over a pointer to PyObject, so a
    // Vec<PyObjectRef> has a layout-compatible contiguous backing buffer. The
    // vector is kept alive for the duration of the call.
    let fastcall_arg_ptrs = fastcall_args.as_ptr().cast::<*mut PyObject>();
    let ret_ptr = unsafe { f(slf_ptr, fastcall_arg_ptrs, nargs as isize, kwnames_ptr) };
    ret_ptr_to_pyresult(vm, ret_ptr)
}

unsafe fn call_fast_function(
    vm: &VirtualMachine,
    method: PyMethodPointer,
    flags: PyMethodFlags,
    args: PosArgs,
) -> PyResult {
    let f = unsafe { method.PyCFunctionFast };
    let mut args: FuncArgs = args.into();
    let slf = take_self_arg(&mut args, flags);
    let slf_ptr = slf
        .as_ref()
        .map(|obj| obj.as_object().as_raw().cast_mut())
        .unwrap_or_default();
    // SAFETY: PyObjectRef is repr(transparent) over a pointer to PyObject, so a
    // Vec<PyObjectRef> has a layout-compatible contiguous backing buffer. The
    // vector is kept alive for the duration of the call.
    let fastcall_arg_ptrs = args.args.as_mut_ptr().cast::<*mut PyObject>();
    let ret_ptr = unsafe { f(slf_ptr, fastcall_arg_ptrs, args.args.len() as isize) };
    ret_ptr_to_pyresult(vm, ret_ptr)
}

fn ret_ptr_to_pyresult(vm: &VirtualMachine, ret_ptr: *mut PyObject) -> PyResult {
    let ret_ptr = NonNull::new(ret_ptr).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("Native function returned NULL, but there was no exception set")
    })?;
    Ok(unsafe { PyObjectRef::from_raw(ret_ptr) })
}

fn take_self_arg(args: &mut FuncArgs, flags: PyMethodFlags) -> Option<PyObjectRef> {
    if flags.contains(PyMethodFlags::STATIC) {
        None
    } else {
        args.take_positional()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCMethod_New(
    ml: *mut PyMethodDef,
    slf: *mut PyObject,
    _module: *mut PyObject,
    _cls: *mut PyTypeObject,
) -> *mut PyObject {
    with_vm(|vm| -> PyResult {
        assert!(
            _cls.is_null(),
            "PyCMethod_New does not support METH_METHOD on abi3"
        );
        let ml = unsafe { &*ml };
        let zelf = unsafe { slf.as_ref().map(|obj| obj.to_owned()) };
        Ok(build_method_def(vm, ml, zelf.is_some())?
            .build_function(vm, zelf)
            .into())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_New(
    ml: *mut PyMethodDef,
    slf: *mut PyObject,
) -> *mut PyObject {
    unsafe { PyCMethod_New(ml, slf, core::ptr::null_mut(), core::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCFunction_NewEx(
    ml: *mut PyMethodDef,
    slf: *mut PyObject,
    module: *mut PyObject,
) -> *mut PyObject {
    unsafe { PyCMethod_New(ml, slf, module, core::ptr::null_mut()) }
}

#[cfg(false)]
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
                assert!(_args.is_null());
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

            assert!(py_fn.call((1,), None).is_err());
            assert!(py_fn.call((1, 2), None).is_err());
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
