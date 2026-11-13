use crate::PyObject;
use crate::object::PyTypeObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::util::{CStrExt, FfiPtrExt};
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

    pub(crate) fn build_method_def(&self, vm: &VirtualMachine) -> PyResult<PyRef<HeapMethodDef>> {
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
                let callable = move |vm: &VirtualMachine, receiver: Option<&rustpython_vm::PyObject>, args: FuncArgs, callee: Callee| {
                    let receiver = receiver.filter(|_| !flags.contains(PyMethodFlags::STATIC));
                    args.bind_for::<()>(vm, callee)?;
                    let slf_ptr = self_ptr(receiver);
                    unsafe {
                        (method.PyCFunction)(slf_ptr, core::ptr::null_mut()).assume_owned_or_err(vm)
                    }
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            PyMethodFlags::VARARGS => {
                let callable = move |vm: &VirtualMachine, receiver: Option<&rustpython_vm::PyObject>, args: FuncArgs, callee: Callee| {
                    let receiver = receiver.filter(|_| !flags.contains(PyMethodFlags::STATIC));
                    let (args,) = args.bind_for::<(PosArgs,)>(vm, callee)?;
                    unsafe { call_function(vm, method, args.into(), receiver) }
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            PyMethodFlags::VARARGS | PyMethodFlags::KEYWORDS => {
                let callable = move |vm: &VirtualMachine, receiver: Option<&rustpython_vm::PyObject>, args: FuncArgs, _callee: Callee| unsafe {
                    let receiver = receiver.filter(|_| !flags.contains(PyMethodFlags::STATIC));
                    call_function_with_keywords(vm, method, args, receiver)
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            PyMethodFlags::FASTCALL | PyMethodFlags::KEYWORDS => {
                let callable = move |vm: &VirtualMachine, receiver: Option<&rustpython_vm::PyObject>, args: FuncArgs, _callee: Callee| unsafe {
                    let receiver = receiver.filter(|_| !flags.contains(PyMethodFlags::STATIC));
                    call_fast_function_with_keywords(vm, method, args, receiver)
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            PyMethodFlags::FASTCALL => {
                let callable = move |vm: &VirtualMachine, receiver: Option<&rustpython_vm::PyObject>, args: FuncArgs, callee: Callee| {
                    let receiver = receiver.filter(|_| !flags.contains(PyMethodFlags::STATIC));
                    let (args,) = args.bind_for::<(PosArgs,)>(vm, callee)?;
                    unsafe { call_fast_function(vm, method, args, receiver) }
                };
                Ok(vm.ctx.new_raw_method_def(name, callable, flags, doc))
            },
            PyMethodFlags::O => {
                let callable = move |vm: &VirtualMachine, receiver: Option<&rustpython_vm::PyObject>, args: FuncArgs, callee: Callee| {
                    let receiver = receiver.filter(|_| !flags.contains(PyMethodFlags::STATIC));
                    let (arg,) = args.bind_for::<(PyObjectRef,)>(vm, callee)?;
                    let slf_ptr = self_ptr(receiver);
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
    args: FuncArgs,
    receiver: Option<&rustpython_vm::PyObject>,
) -> PyResult {
    let f = unsafe { method.PyCFunction };
    let arg_tuple = vm.ctx.new_tuple(args.args);
    let slf_ptr = self_ptr(receiver);
    unsafe { f(slf_ptr, arg_tuple.as_object().as_raw().cast_mut()).assume_owned_or_err(vm) }
}

unsafe fn call_function_with_keywords(
    vm: &VirtualMachine,
    method: PyMethodPointer,
    args: FuncArgs,
    receiver: Option<&rustpython_vm::PyObject>,
) -> PyResult {
    let f = unsafe { method.PyCFunctionWithKeywords };
    let slf_ptr = self_ptr(receiver);
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
    args: FuncArgs,
    receiver: Option<&rustpython_vm::PyObject>,
) -> PyResult {
    let f = unsafe { method.PyCFunctionFastWithKeywords };
    let slf_ptr = self_ptr(receiver);
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
    args: PosArgs,
    receiver: Option<&rustpython_vm::PyObject>,
) -> PyResult {
    let f = unsafe { method.PyCFunctionFast };
    let args: FuncArgs = args.into();
    let slf_ptr = self_ptr(receiver);
    // SAFETY: PyObjectRef is repr(transparent) over a pointer to PyObject, so a
    // Vec<PyObjectRef> has a layout-compatible contiguous backing buffer. The
    // vector is kept alive for the duration of the call.
    let fastcall_arg_ptrs = args.args.as_ptr().cast::<*mut PyObject>();
    unsafe { f(slf_ptr, fastcall_arg_ptrs, args.args.len() as isize).assume_owned_or_err(vm) }
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
        Ok(ml.build_method_def(vm)?.build_function(vm, zelf).into())
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
    #[test]
    fn native_methods_receive_self_separately_from_user_arguments() {
        use pyo3::ffi;
        use pyo3::types::{PyDict, PyTuple};

        unsafe extern "C" fn receiver(slf: *mut PyObject, _args: *mut PyObject) -> *mut PyObject {
            unsafe { ffi::Py_NewRef(slf) }
        }
        unsafe extern "C" fn one(_slf: *mut PyObject, arg: *mut PyObject) -> *mut PyObject {
            unsafe { ffi::Py_NewRef(arg) }
        }
        unsafe extern "C" fn variadic(
            slf: *mut PyObject,
            args: *mut PyObject,
            kwargs: *mut PyObject,
        ) -> *mut PyObject {
            unsafe {
                let result = ffi::PyTuple_New(3);
                if result.is_null() {
                    return result;
                }
                let kwargs = if kwargs.is_null() {
                    ffi::Py_None()
                } else {
                    kwargs
                };
                ffi::PyTuple_SetItem(result, 0, ffi::Py_NewRef(slf));
                ffi::PyTuple_SetItem(result, 1, ffi::Py_NewRef(args));
                ffi::PyTuple_SetItem(result, 2, ffi::Py_NewRef(kwargs));
                result
            }
        }
        unsafe extern "C" fn static_noargs(
            _slf: *mut PyObject,
            _args: *mut PyObject,
        ) -> *mut PyObject {
            unsafe { PyLong_FromLong(42) }
        }

        Python::attach(|py| {
            let method = |name: &'static core::ffi::CStr, function, flags| ffi::PyMethodDef {
                ml_name: name.as_ptr(),
                ml_meth: ffi::PyMethodDefPointer {
                    PyCFunction: function,
                },
                ml_flags: flags,
                ml_doc: c"".as_ptr(),
            };
            let mut methods = [
                method(c"noargs", receiver as ffi::PyCFunction, ffi::METH_NOARGS),
                method(c"one", one as ffi::PyCFunction, ffi::METH_O),
                ffi::PyMethodDef {
                    ml_name: c"variadic".as_ptr(),
                    ml_meth: ffi::PyMethodDefPointer {
                        PyCFunctionWithKeywords: variadic,
                    },
                    ml_flags: ffi::METH_VARARGS | ffi::METH_KEYWORDS,
                    ml_doc: c"".as_ptr(),
                },
                method(
                    c"class_receiver",
                    receiver as ffi::PyCFunction,
                    ffi::METH_CLASS | ffi::METH_NOARGS,
                ),
                method(
                    c"static_noargs",
                    static_noargs as ffi::PyCFunction,
                    ffi::METH_STATIC | ffi::METH_NOARGS,
                ),
                method(
                    c"explicit_type_argument",
                    one as ffi::PyCFunction,
                    ffi::METH_STATIC | ffi::METH_O,
                ),
                ffi::PyMethodDef::zeroed(),
            ];
            // This boundary does not need a Rust payload. Use a zero-sized C
            // type spec instead of PyO3's currently unsupported payload layout.
            let mut slots = [
                ffi::PyType_Slot {
                    slot: ffi::Py_tp_methods,
                    pfunc: methods.as_mut_ptr().cast(),
                },
                ffi::PyType_Slot {
                    slot: 0,
                    pfunc: core::ptr::null_mut(),
                },
            ];
            let mut spec = ffi::PyType_Spec {
                name: c"test.Receiver".as_ptr(),
                basicsize: 0,
                itemsize: 0,
                flags: ffi::Py_TPFLAGS_DEFAULT as _,
                slots: slots.as_mut_ptr(),
            };
            let class = unsafe {
                Bound::<PyAny>::from_owned_ptr_or_err(py, ffi::PyType_FromSpec(&mut spec))
            }
            .unwrap();
            let obj = class.call0().unwrap();
            let bound = obj.getattr("one").unwrap();
            assert!(bound.getattr("__self__").unwrap().is(&obj));
            assert_eq!(bound.call1((2,)).unwrap().extract::<i64>().unwrap(), 2);
            assert_eq!(
                class
                    .getattr("one")
                    .unwrap()
                    .call1((&obj, 2))
                    .unwrap()
                    .extract::<i64>()
                    .unwrap(),
                2
            );
            assert!(bound.call0().is_err());
            assert!(bound.call1((1, 2)).is_err());
            assert!(class.getattr("one").unwrap().call0().is_err());
            assert!(class.getattr("one").unwrap().call1((1, 2)).is_err());
            assert!(obj.call_method0("noargs").unwrap().is(&obj));
            let kwargs = PyDict::new(py);
            kwargs.set_item("named", 3).unwrap();
            let result = obj.call_method("variadic", (1, 2), Some(&kwargs)).unwrap();
            let result = result.cast::<PyTuple>().unwrap();
            assert!(result.get_item(0).unwrap().is(&obj));
            assert_eq!(
                result.get_item(1).unwrap().extract::<(i64, i64)>().unwrap(),
                (1, 2)
            );
            assert_eq!(
                result
                    .get_item(2)
                    .unwrap()
                    .get_item("named")
                    .unwrap()
                    .extract::<i64>()
                    .unwrap(),
                3
            );
            let without_keywords = obj.call_method1("variadic", (1, 2)).unwrap();
            assert!(
                without_keywords
                    .cast::<PyTuple>()
                    .unwrap()
                    .get_item(2)
                    .unwrap()
                    .is_none()
            );
            assert!(obj.call_method0("class_receiver").unwrap().is(&class));
            assert_eq!(
                class
                    .call_method0("static_noargs")
                    .unwrap()
                    .extract::<i64>()
                    .unwrap(),
                42
            );
            assert_eq!(
                obj.call_method0("static_noargs")
                    .unwrap()
                    .extract::<i64>()
                    .unwrap(),
                42
            );
            let static_function = class.getattr("explicit_type_argument").unwrap();
            assert!(static_function.getattr("__self__").unwrap().is_none());
            let wrapper = class
                .getattr("__dict__")
                .unwrap()
                .get_item("explicit_type_argument")
                .unwrap();
            assert_eq!(
                wrapper.get_type().name().unwrap().to_str().unwrap(),
                "staticmethod"
            );
            assert!(wrapper.getattr("__func__").unwrap().is(&static_function));
            let module = static_function.getattr("__module__").unwrap();
            class
                .setattr("__module__", "changed_after_registration")
                .unwrap();
            assert!(static_function.getattr("__module__").unwrap().is(&module));
            assert!(static_function.call1((&class,)).unwrap().is(&class));
            assert!(wrapper.call1((&class,)).unwrap().is(&class));
            assert!(
                obj.call_method1("explicit_type_argument", (&class,))
                    .unwrap()
                    .is(&class)
            );
            assert!(obj.call_method0("explicit_type_argument").is_err());
        });
    }
    #[test]
    fn static_c_adapter_ignores_its_bound_class() {
        unsafe extern "C" fn check_self(
            slf: *mut crate::PyObject,
            args: *mut crate::PyObject,
        ) -> *mut crate::PyObject {
            unsafe { PyLong_FromLong((slf.is_null() && args.is_null()).into()).cast() }
        }
        Python::attach(|py| {
            let mut definition = super::PyMethodDef {
                ml_name: c"static_noargs".as_ptr(),
                ml_meth: super::PyMethodPointer {
                    PyCFunction: check_self,
                },
                ml_flags: pyo3::ffi::METH_STATIC | pyo3::ffi::METH_NOARGS,
                ml_doc: c"".as_ptr(),
            };
            // The safe PyO3 wrapper owns the function; only construction crosses
            // the C boundary so the callback can observe its actual self pointer.
            let function = unsafe {
                Bound::<PyAny>::from_owned_ptr_or_err(
                    py,
                    super::PyCFunction_New(&mut definition, py.get_type::<PyAny>().as_ptr().cast())
                        .cast(),
                )
            }
            .unwrap();
            assert!(function.getattr("__self__").unwrap().is_none());
            assert_eq!(function.call0().unwrap().extract::<i64>().unwrap(), 1);
        });
    }
}
