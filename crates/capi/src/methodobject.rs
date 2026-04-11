use crate::PyObject;
use crate::object::PyTypeObject;
use crate::pystate::with_vm;
use core::ffi::{CStr, c_char, c_int, c_void};
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
pub struct PyMethodDef {
    pub ml_name: *const c_char,
    pub ml_meth: *mut c_void,
    pub ml_flags: c_int,
    pub ml_doc: *const c_char,
}

fn c_function_wrapper(
    vm: &VirtualMachine,
    slf: &PyObjectRef,
    mut args: FuncArgs,
    ml_meth: usize,
    ml_flags: c_int,
) -> PyResult {
    let slf = slf.as_object().as_raw().cast_mut();
    let flags = PyMethodFlags::from_bits_truncate(ml_flags as u32);

    let arg_tuple = vm.ctx.new_tuple(core::mem::take(&mut args.args));
    let arg_tuple_ptr = arg_tuple.as_object().as_raw().cast_mut();

    let ret_ptr = if flags.contains(PyMethodFlags::KEYWORDS) {
        let kwargs = vm.ctx.new_dict();
        for (k, v) in args.kwargs {
            kwargs.set_item(&*k, v, vm)?;
        }
        let kwargs_ptr = kwargs.as_object().as_raw().cast_mut();
        let f = unsafe { core::mem::transmute::<usize, PyCFunctionWithKeywords>(ml_meth) };
        unsafe { f(slf, arg_tuple_ptr, kwargs_ptr) }
    } else {
        let f = unsafe { core::mem::transmute::<usize, PyCFunction>(ml_meth) };
        unsafe { f(slf, arg_tuple_ptr) }
    };

    let ret_ptr = NonNull::new(ret_ptr)
        .ok_or_else(|| vm.new_system_error("native method returned NULL".to_owned()))?;
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

        let ml_meth = ml.ml_meth as usize;
        let ml_flags = ml.ml_flags;
        let slf = unsafe { &*slf }.to_owned();
        let callable = move |args: FuncArgs, vm: &VirtualMachine| {
            c_function_wrapper(vm, &slf, args, ml_meth, ml_flags)
        };

        let flags = PyMethodFlags::from_bits_truncate(ml_flags as u32);
        let method = vm.ctx.new_method_def(name, callable, flags, Some(doc));
        Ok(method.build_function(vm).into())
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyCFunction, PyString};

    #[test]
    fn test_new_c_function() {
        Python::attach(|py| {
            let f = PyCFunction::new_closure(py, None, None, |_args, _kwargs| "Hello from Rust!")
                .unwrap();

            assert_eq!(
                f.call0().unwrap().cast::<PyString>().unwrap(),
                "Hello from Rust!"
            );
        })
    }
}
