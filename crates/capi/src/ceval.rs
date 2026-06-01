use crate::pystate::with_vm;
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use rustpython_vm::builtins::{PyCode, PyDict};
use rustpython_vm::compiler::Mode;
use rustpython_vm::function::ArgMapping;
use rustpython_vm::scope::Scope;
use rustpython_vm::{AsObject, PyObject, TryFromObject};

const PY_SINGLE_INPUT: c_int = 256;
const PY_FILE_INPUT: c_int = 257;
const PY_EVAL_INPUT: c_int = 258;
const PY_FUNC_TYPE_INPUT: c_int = 345;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_CompileString(
    code: *const c_char,
    filename: *const c_char,
    start: c_int,
) -> *mut PyObject {
    with_vm(|vm| {
        let code = unsafe { CStr::from_ptr(code) }.to_str().map_err(|_| {
            vm.new_system_error("Py_CompileString called with non UTF-8 code string")
        })?;
        let filename = unsafe { CStr::from_ptr(filename) }
            .to_str()
            .map_err(|_| vm.new_system_error("Py_CompileString called with non UTF-8 filename"))?;

        let mode = match start {
            PY_SINGLE_INPUT => Mode::Single,
            PY_FILE_INPUT => Mode::Exec,
            PY_EVAL_INPUT => Mode::Eval,
            PY_FUNC_TYPE_INPUT => Mode::BlockExpr,
            _ => {
                return Err(
                    vm.new_system_error("Invalid start argument passed to Py_CompileString")
                );
            }
        };

        vm.compile(code, mode, filename.to_owned())
            .map_err(|err| vm.new_syntax_error(&err, Some(code)))
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
pub extern "C" fn PyEval_GetBuiltins() -> *mut PyObject {
    with_vm(|vm| {
        vm.current_frame().map_or_else(
            || vm.builtins.as_object().as_raw(),
            |frame| frame.builtins.as_object().as_raw(),
        )
    })
}

#[cfg(false)]
mod tests {
    use pyo3::exceptions::PyException;
    use pyo3::prelude::*;

    #[test]
    fn test_code_eval() {
        Python::attach(|py| {
            let result = py.eval(c"1 + 1", None, None).unwrap();
            assert_eq!(result.extract::<u32>().unwrap(), 2);
        })
    }

    #[test]
    fn test_code_run_exception() {
        Python::attach(|py| {
            let err = py.run(c"raise Exception()", None, None).unwrap_err();
            assert!(err.is_instance_of::<PyException>(py));
        })
    }
}
