use crate::PyObject;
use crate::object::PyTypeObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::util::{CStrExt, FfiPtrExt};
use alloc::borrow::Cow;
use core::ffi::{c_char, c_int};
use core::fmt::Debug;
use rustpython_vm::function::{Callee, FuncArgs, HeapMethodDef, ItemDoc, PosArgs, PyMethodFlags};
use rustpython_vm::types::c_slots::{kwargs_ptr, split_args};
use rustpython_vm::{AsObject, PyObjectRef, PyRef, PyResult, VirtualMachine};

define_py_check!(fn PyCFunction_Check, types.builtin_function_or_method_type);
define_py_check!(exact fn PyCFunction_CheckExact, types.builtin_function_or_method_type);

#[repr(C)]
#[derive(Debug)]
pub struct PyMethodDef {
    pub ml_name: *const c_char,
    pub ml_meth: PyMethodPointer,
    pub ml_flags: c_int,
    pub ml_doc: *const c_char,
}

impl PyMethodDef {
    pub(crate) fn iter<'a>(mut methods: *const Self) -> impl Iterator<Item = &'a Self> {
        core::iter::from_fn(move || {
            let def = unsafe { &*methods };
            if def.ml_name.is_null() {
                None
            } else {
                methods = unsafe { methods.add(1) };
                Some(def)
            }
        })
    }

    pub(crate) fn build_method_def(
        &self,
        vm: &VirtualMachine,
        has_self: bool,
    ) -> PyResult<PyRef<HeapMethodDef>> {
        let name = unsafe { self.ml_name.try_as_str(vm) }?;

        let doc = unsafe { self.ml_doc.try_as_str_opt(vm) }?;
        let doc = doc
            .filter(|doc| !doc.is_empty())
            .map_or(ItemDoc::NONE, |doc| {
                let text: &'static str = Box::leak(doc.to_owned().into_boxed_str());
                ItemDoc::static_text(text)
            });

        let flags = PyMethodFlags::from_bits(self.ml_flags as u32)
            .ok_or_else(|| vm.new_system_error("PyMethodDef contains unknown flags"))?;
        let has_self = has_self && !flags.contains(PyMethodFlags::STATIC);

        let method = self.ml_meth;

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
                let callable = move |vm: &VirtualMachine, mut args: FuncArgs, callee: Callee<'_>| {
                    let slf = take_self_arg(&mut args, has_self, callee);
                    args.bind_for::<()>(vm, callee.with_instance_arg(false))?;
                    let slf_ptr = self_ptr(slf.as_deref());
                    unsafe {
                        (method.PyCFunction)(slf_ptr, core::ptr::null_mut()).assume_owned_or_err(vm)
                    }
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            PyMethodFlags::VARARGS => {
                let callable = move |vm: &VirtualMachine, args: FuncArgs, callee: Callee<'_>| {
                    let (args,) = args.bind_for::<(PosArgs,)>(vm, callee)?;
                    unsafe { call_function(vm, method, has_self, args.into(), callee) }
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            PyMethodFlags::VARARGS | PyMethodFlags::KEYWORDS => {
                let callable = move |vm: &VirtualMachine, args: FuncArgs, callee: Callee<'_>| unsafe {
                    call_function_with_keywords(vm, method, has_self, args, callee)
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            PyMethodFlags::FASTCALL | PyMethodFlags::KEYWORDS => {
                let callable = move |vm: &VirtualMachine, args: FuncArgs, callee: Callee<'_>| unsafe {
                    call_fast_function_with_keywords(vm, method, has_self, args, callee)
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            PyMethodFlags::FASTCALL => {
                let callable = move |vm: &VirtualMachine, args: FuncArgs, callee: Callee<'_>| {
                    let (args,) = args.bind_for::<(PosArgs,)>(vm, callee)?;
                    unsafe { call_fast_function(vm, method, has_self, args, callee) }
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            PyMethodFlags::O => {
                let callable = move |vm: &VirtualMachine, mut args: FuncArgs, callee: Callee<'_>| {
                    let slf = take_self_arg(&mut args, has_self, callee);
                    let (arg,) = args.bind_for::<(PyObjectRef,)>(vm, callee.with_instance_arg(false))?;
                    let slf_ptr = self_ptr(slf.as_deref());
                    unsafe {
                        (method.PyCFunction)(slf_ptr, arg.as_raw().cast_mut()).assume_owned_or_err(vm)
                    }
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            _ => {
                Err(vm.new_system_error(format!(
                    "function {name} has unsupported or invalid calling-convention flags: {flags:?}"
                )))
            },
        })
    }
}

pub type PyCFunction =
    unsafe extern "C" fn(slf: *mut PyObject, args: *mut PyObject) -> *mut PyObject;

pub type PyCFunctionWithKeywords = unsafe extern "C" fn(
    slf: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject;

pub type PyCFunctionFast = unsafe extern "C" fn(
    slf: *mut PyObject,
    args: *const *mut PyObject,
    nargs: isize,
) -> *mut PyObject;

pub type PyCFunctionFastWithKeywords = unsafe extern "C" fn(
    slf: *mut PyObject,
    args: *const *mut PyObject,
    nargs: isize,
    kwnames: *mut PyObject,
) -> *mut PyObject;

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(non_snake_case)]
pub union PyMethodPointer {
    pub PyCFunction: PyCFunction,
    pub PyCFunctionWithKeywords: PyCFunctionWithKeywords,
    pub PyCFunctionFast: PyCFunctionFast,
    pub PyCFunctionFastWithKeywords: PyCFunctionFastWithKeywords,
}

impl Debug for PyMethodPointer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        unsafe { self.PyCFunction.fmt(f) }
    }
}

unsafe fn call_function(
    vm: &VirtualMachine,
    method: PyMethodPointer,
    has_self: bool,
    mut args: FuncArgs,
    callee: Callee<'_>,
) -> PyResult {
    let f = unsafe { method.PyCFunction };
    let slf = take_self_arg(&mut args, has_self, callee);
    let arg_tuple = vm.ctx.new_tuple(args.args);
    let slf_ptr = self_ptr(slf.as_deref());
    unsafe { f(slf_ptr, arg_tuple.as_object().as_raw().cast_mut()).assume_owned_or_err(vm) }
}

unsafe fn call_function_with_keywords(
    vm: &VirtualMachine,
    method: PyMethodPointer,
    has_self: bool,
    mut args: FuncArgs,
    callee: Callee<'_>,
) -> PyResult {
    let f = unsafe { method.PyCFunctionWithKeywords };
    let slf = take_self_arg(&mut args, has_self, callee);
    let slf_ptr = self_ptr(slf.as_deref());
    let (arg_tuple, kwargs) = split_args(vm, args)?;
    unsafe {
        f(
            slf_ptr,
            arg_tuple.as_object().as_raw().cast_mut(),
            kwargs_ptr(kwargs.as_ref()),
        )
        .assume_owned_or_err(vm)
    }
}

unsafe fn call_fast_function_with_keywords(
    vm: &VirtualMachine,
    method: PyMethodPointer,
    has_self: bool,
    mut args: FuncArgs,
    callee: Callee<'_>,
) -> PyResult {
    let f = unsafe { method.PyCFunctionFastWithKeywords };
    let slf = take_self_arg(&mut args, has_self, callee);
    let slf_ptr = self_ptr(slf.as_deref());
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
        .map_or_default(|tuple| tuple.as_object().as_raw().cast_mut());
    // SAFETY: PyObjectRef is repr(transparent) over a pointer to PyObject, so a
    // Vec<PyObjectRef> has a layout-compatible contiguous backing buffer. The
    // vector is kept alive for the duration of the call.
    let fastcall_arg_ptrs = fastcall_args.as_ptr().cast::<*mut PyObject>();
    unsafe { f(slf_ptr, fastcall_arg_ptrs, nargs as isize, kwnames_ptr).assume_owned_or_err(vm) }
}

unsafe fn call_fast_function(
    vm: &VirtualMachine,
    method: PyMethodPointer,
    has_self: bool,
    args: PosArgs,
    callee: Callee<'_>,
) -> PyResult {
    let f = unsafe { method.PyCFunctionFast };
    let mut args: FuncArgs = args.into();
    let slf = take_self_arg(&mut args, has_self, callee);
    let slf_ptr = self_ptr(slf.as_deref());
    // SAFETY: PyObjectRef is repr(transparent) over a pointer to PyObject, so a
    // Vec<PyObjectRef> has a layout-compatible contiguous backing buffer. The
    // vector is kept alive for the duration of the call.
    let fastcall_arg_ptrs = args.args.as_mut_ptr().cast::<*mut PyObject>();
    unsafe { f(slf_ptr, fastcall_arg_ptrs, args.args.len() as isize).assume_owned_or_err(vm) }
}

fn take_self_arg<'a>(
    args: &mut FuncArgs,
    has_self: bool,
    callee: Callee<'a>,
) -> Option<Cow<'a, rustpython_vm::PyObject>> {
    if !has_self {
        None
    } else if let Some(module) = callee.module() {
        Some(Cow::Borrowed(module.as_object()))
    } else {
        args.take_positional().map(Cow::Owned)
    }
}

fn self_ptr(slf: Option<&rustpython_vm::PyObject>) -> *mut PyObject {
    slf.map_or(core::ptr::null_mut(), |obj| obj.as_raw().cast_mut())
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
        let zelf = unsafe { slf.assume_borrowed_or_opt() }.map(ToOwned::to_owned);
        Ok(ml
            .build_method_def(vm, zelf.is_some())?
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

#[cfg(test)]
mod tests {
    use pyo3::exceptions::PyException;
    use pyo3::ffi::{PyLong_FromLong, PyObject};
    use pyo3::prelude::*;
    use pyo3::types::{PyCFunction, PyInt, PyString};

    #[test]
    fn closure_function() {
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
    fn function_no_args() {
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
    fn closure_function_error() {
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

    #[test]
    fn wrap_static_no_args_function() {
        #[pyfunction()]
        fn f() {}

        Python::attach(|py| {
            let module = PyModule::new(py, "test_wrap_pyfunction_forms").unwrap();

            let func = wrap_pyfunction!(f, &module).unwrap();
            func.call0().unwrap();
        });
    }

    #[test]
    fn wrap_one_arg_function_with_no_module() {
        #[pyfunction]
        fn one_arg(value: i64) -> i64 {
            value
        }

        Python::attach(|py| {
            let func = wrap_pyfunction!(one_arg, py).unwrap();
            func.call1((7,)).unwrap();
        });
    }

    #[test]
    fn module_receiver_is_not_a_python_argument() {
        #[pyfunction(pass_module)]
        fn module_noargs(module: &Bound<'_, PyModule>) -> Py<PyModule> {
            module.clone().unbind()
        }

        #[pyfunction(pass_module, signature = (value, /))]
        fn module_one(module: &Bound<'_, PyModule>, value: i64) -> (Py<PyModule>, i64) {
            (module.clone().unbind(), value)
        }

        #[pyfunction(pass_module, signature = (*args, **kwargs))]
        fn module_variadic(
            module: &Bound<'_, PyModule>,
            args: &Bound<'_, pyo3::types::PyTuple>,
            kwargs: Option<&Bound<'_, pyo3::types::PyDict>>,
        ) -> (Py<PyModule>, usize, usize) {
            (
                module.clone().unbind(),
                args.len(),
                kwargs.map_or(0, |kwargs| kwargs.len()),
            )
        }

        Python::attach(|py| {
            let module = PyModule::new(py, "receiver_module").unwrap();
            let noargs = wrap_pyfunction!(module_noargs, &module).unwrap();
            assert!(noargs.getattr("__self__").unwrap().is(&module));
            assert!(noargs.call0().unwrap().is(&module));
            assert!(noargs.call1((1,)).is_err());

            let one = wrap_pyfunction!(module_one, &module).unwrap();
            let (receiver, value): (Py<PyModule>, i64) =
                one.call1((7,)).unwrap().extract().unwrap();
            assert!(receiver.bind(py).is(&module));
            assert_eq!(value, 7);
            assert!(one.call0().is_err());
            assert!(one.call1((7, 8)).is_err());

            let variadic = wrap_pyfunction!(module_variadic, &module).unwrap();
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("keyword", 3).unwrap();
            let (receiver, positional, keywords): (Py<PyModule>, usize, usize) = variadic
                .call((1, 2), Some(&kwargs))
                .unwrap()
                .extract()
                .unwrap();
            assert!(receiver.bind(py).is(&module));
            assert_eq!((positional, keywords), (2, 1));
        });
    }

    #[test]
    fn no_keywords_before_arity() {
        Python::attach(|py| {
            unsafe extern "C" fn c_fn(_self: *mut PyObject, _args: *mut PyObject) -> *mut PyObject {
                unsafe { PyLong_FromLong(42) }
            }
            let f = PyCFunction::new(py, c_fn, c"no_keywords", c"", None).unwrap();
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("unexpected", 1).unwrap();
            let err = f.call((1,), Some(&kwargs)).unwrap_err();
            assert_eq!(
                err.value(py).str().unwrap().to_str().unwrap(),
                "no_keywords() takes no keyword arguments"
            );
            kwargs.clear();
            assert_eq!(
                f.call((), Some(&kwargs)).unwrap().extract::<i32>().unwrap(),
                42
            );
        });
    }
}
