use crate::pyframe::PyFrameObject;
use crate::pystate::with_vm;
use crate::unicodeobject::decode_fsdefault_and_size;
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use rustpython_vm::builtins::{PyCode, PyDict};
use rustpython_vm::function::ArgMapping;
use rustpython_vm::scope::Scope;
use rustpython_vm::{AsObject, PyObject, TryFromObject};
use rustpython_vm::{PyObjectRef, version};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_CompileString(
    code: *const c_char,
    filename: *const c_char,
    start: c_int,
) -> *mut PyObject {
    with_vm(|vm| {
        let code = unsafe { CStr::from_ptr(code) }.to_bytes();
        let filename_size = unsafe { CStr::from_ptr(filename) }.to_bytes().len();
        let filename = decode_fsdefault_and_size(vm, filename, filename_size)?;
        let filename = filename.to_string_lossy();
        vm.compile_string_object_with_flags(code, &filename, start, 0, version::MINOR as c_int, -1)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalCode(
    co: *mut PyObject,
    globals: *mut PyObject,
    locals: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let code = unsafe { &*co }.try_downcast_ref::<PyCode>(vm)?;
        let globals = unsafe { &*globals }.try_downcast_ref::<PyDict>(vm)?;
        let locals = NonNull::new(locals)
            .map(|ptr| ArgMapping::try_from_object(vm, unsafe { ptr.as_ref() }.to_owned()))
            .transpose()?;

        let scope = Scope::with_builtins(locals, globals.to_owned(), vm);

        vm.run_code_obj(code.to_owned(), scope)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalFrame(f: *mut PyFrameObject) -> *mut PyObject {
    unsafe { PyEval_EvalFrameEx(f, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalFrameEx(f: *mut PyFrameObject, _exc: c_int) -> *mut PyObject {
    with_vm(|vm| vm.run_frame(unsafe { &*f }.to_owned()))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetBuiltins() -> *mut PyObject {
    with_vm(|vm| {
        vm.current_frame().map_or_else(
            || vm.builtins.as_object().as_raw(),
            |frame| frame.iframe().builtins().as_raw(),
        )
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetFrame() -> *mut PyFrameObject {
    with_vm(|vm| -> *mut PyObject {
        vm.current_frame()
            .map(|frame| frame.as_object().as_raw().cast_mut())
            .unwrap_or_default()
    })
    .cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetFrameBuiltins() -> *mut PyObject {
    with_vm(|vm| {
        vm.current_frame().map_or_else(
            || vm.builtins.as_object().to_owned(),
            |frame| frame.iframe().builtins().to_owned(),
        )
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetFrameGlobals() -> *mut PyObject {
    with_vm(|vm| {
        vm.current_frame()
            .map(|frame| {
                frame
                    .iframe()
                    .globals()
                    .as_object()
                    .to_owned()
                    .into_raw()
                    .as_ptr()
            })
            .unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetFrameLocals() -> *mut PyObject {
    with_vm(|vm| {
        let Some(frame) = vm.current_frame() else {
            return Ok(core::ptr::null_mut());
        };
        let locals: PyObjectRef = frame.locals(vm)?.into();
        Ok(locals.into_raw().as_ptr())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetGlobals() -> *mut PyObject {
    with_vm(|vm| {
        vm.current_frame()
            .map(|frame| frame.iframe().globals().as_object().as_raw())
            .unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetLocals() -> *mut PyObject {
    with_vm(|vm| {
        let Some(frame) = vm.current_frame() else {
            return Ok(core::ptr::null_mut());
        };
        let locals = frame.locals(vm)?;
        if frame
            .iframe()
            .locals
            .get()
            .is_some_and(|mapping| mapping.obj().is(locals.obj()))
        {
            return Ok(locals.obj().as_raw().cast_mut());
        }
        let cache = frame.locals_snapshot_cache(vm);
        cache.merge_object(locals.into(), vm)?;
        Ok(cache.as_object().as_raw().cast_mut())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_GetFuncDesc(func: *mut PyObject) -> *const c_char {
    with_vm(|vm| {
        let func = unsafe { &*func };
        let cls = func.class();
        if cls.is(vm.ctx.types.bound_method_type)
            || cls.is(vm.ctx.types.function_type)
            || cls.is(vm.ctx.types.builtin_function_or_method_type)
        {
            c"()"
        } else {
            c" object"
        }
    })
}

#[cfg(test)]
mod tests {
    use alloc::ffi::CString;
    use pyo3::exceptions::PyException;
    use pyo3::prelude::*;

    #[pyfunction]
    fn legacy_local(name: &str) -> (usize, Option<i64>) {
        let locals = super::PyEval_GetLocals();
        assert!(!locals.is_null());
        let name = CString::new(name).unwrap();
        let value = unsafe { crate::dictobject::PyDict_GetItemString(locals, name.as_ptr()) };
        let value = if value.is_null() {
            None
        } else {
            Some(unsafe { crate::longobject::PyLong_AsLongLong(value) })
        };
        (locals as usize, value)
    }

    #[pyfunction]
    fn legacy_locals_is_frame_mapping() -> bool {
        let locals = super::PyEval_GetLocals();
        rustpython_vm::vm::thread::with_current_vm(|vm| {
            let frame = vm.current_frame().unwrap();
            core::ptr::eq(
                locals.cast_const(),
                frame.iframe().locals.as_object(vm).as_raw(),
            )
        })
    }

    #[pyfunction]
    fn frame_locals_are_fresh() -> bool {
        let first = super::PyEval_GetFrameLocals();
        let second = super::PyEval_GetFrameLocals();
        assert!(!first.is_null() && !second.is_null());
        let different = first != second;
        unsafe {
            crate::refcount::_Py_DecRef(first);
            crate::refcount::_Py_DecRef(second);
        }
        different
    }

    #[test]
    fn code_eval() {
        Python::attach(|py| {
            let result = py.eval(c"1 + 1", None, None).unwrap();
            assert_eq!(result.extract::<u32>().unwrap(), 2);
        })
    }

    #[test]
    fn code_run_exception() {
        Python::attach(|py| {
            let err = py.run(c"raise Exception()", None, None).unwrap_err();
            assert!(err.is_instance_of::<PyException>(py));
        })
    }

    #[test]
    fn legacy_locals_cache_is_lazy_and_does_not_modify_namespaces() {
        Python::attach(|py| {
            let globals = pyo3::types::PyDict::new(py);
            globals
                .set_item("legacy_local", wrap_pyfunction!(legacy_local, py).unwrap())
                .unwrap();
            globals
                .set_item(
                    "legacy_locals_is_frame_mapping",
                    wrap_pyfunction!(legacy_locals_is_frame_mapping, py).unwrap(),
                )
                .unwrap();
            globals
                .set_item(
                    "frame_locals_are_fresh",
                    wrap_pyfunction!(frame_locals_are_fresh, py).unwrap(),
                )
                .unwrap();
            py.run(
                c"\
assert legacy_locals_is_frame_mapping()

def optimized():
    x = 1
    first_ptr, first_x = legacy_local('x')
    x = 2
    second_ptr, second_x = legacy_local('x')
    return first_ptr == second_ptr, first_x, second_x, frame_locals_are_fresh()

def optimized_with_custom_locals():
    x = 3
    legacy_local('x')

optimized_result = optimized()
custom_locals = {}
exec(optimized_with_custom_locals.__code__, globals(), custom_locals)
hidden_result = [legacy_local('hidden_name')[1] for hidden_name in [7]][0]
hidden_leaked = 'hidden_name' in globals()
assert optimized_result == (True, 1, 2, True)
assert custom_locals == {}
assert hidden_result == 7
assert not hidden_leaked
",
                Some(&globals),
                None,
            )
            .unwrap();
        })
    }
}
