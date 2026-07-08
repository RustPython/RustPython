use crate::pyframe::PyFrameObject;
use crate::pystate::with_vm;
use crate::unicodeobject::decode_fsdefault_and_size;
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use rustpython_vm::builtins::{PyCode, PyDict, PyStr};
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
            |frame| frame.builtins.as_object().as_raw(),
        )
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetFrame() -> *mut PyObject {
    with_vm(|vm| {
        vm.current_frame()
            .map(|frame| frame.as_object().as_raw())
            .unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetFrameBuiltins() -> *mut PyObject {
    with_vm(|vm| {
        vm.current_frame().map_or_else(
            || vm.builtins.as_object().to_owned(),
            |frame| frame.builtins.as_object().to_owned(),
        )
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetFrameGlobals() -> *mut PyObject {
    with_vm(|vm| {
        vm.current_frame()
            .map(|frame| frame.globals.as_object().to_owned().into_raw().as_ptr())
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
            .map(|frame| frame.globals.as_object().as_raw())
            .unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetLocals() -> *mut PyObject {
    with_vm(|vm| {
        vm.current_frame()
            .map(|frame| frame.locals.as_object(vm).as_raw())
            .unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_GetFuncName(func: *mut PyObject) -> *const c_char {
    with_vm(|vm| {
        let func = unsafe { &*func };
        let cls = func.class();

        if cls.is(vm.ctx.types.bound_method_type) {
            let function = func.get_attr("__func__", vm)?;
            return Ok(unsafe { PyEval_GetFuncName(function.as_object().as_raw().cast_mut()) });
        }

        let name = if cls.is(vm.ctx.types.function_type)
            || cls.is(vm.ctx.types.builtin_function_or_method_type)
        {
            func.get_attr(rustpython_vm::identifier!(vm, __name__), vm)?
                .downcast_ref::<PyStr>()
                .and_then(|s| s.to_str())
                .map_or_else(|| cls.name().as_ptr(), |s| s.as_ptr())
        } else {
            cls.name().as_ptr()
        };

        Ok(name.cast())
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
    use pyo3::exceptions::PyException;
    use pyo3::prelude::*;

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
}
