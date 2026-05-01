use crate::pystate::with_vm;
use core::ffi::{CStr, c_char, c_int};
use rustpython_vm::builtins::{PyCode, PyDict};
use rustpython_vm::compiler::Mode;
use rustpython_vm::function::ArgMapping;
use rustpython_vm::scope::Scope;
use rustpython_vm::{AsObject, PyObject};

#[unsafe(no_mangle)]
pub extern "C" fn Py_CompileString(
    code: *const c_char,
    filename: *const c_char,
    start: c_int,
) -> *mut PyObject {
    with_vm(|vm| {
        let code = unsafe { CStr::from_ptr(code) }
            .to_str()
            .expect("Invalid UTF-8 in code string");
        let filename = unsafe { CStr::from_ptr(filename) }
            .to_str()
            .expect("Invalid UTF-8 in filename string");

        let mode = match start {
            256 => Mode::Single,
            257 => Mode::Exec,
            258 => Mode::Eval,
            _ => panic!("Invalid start argument to Py_CompileString: {start}"),
        };

        vm.compile(code, mode, filename.to_owned())
            .map_err(|err| vm.new_syntax_error(&err, Some(code)))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_EvalCode(
    co: *mut PyObject,
    globals: *mut PyObject,
    locals: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let code = unsafe { &*co }.try_downcast_ref::<PyCode>(vm)?;
        let globals = unsafe { &*globals }.try_downcast_ref::<PyDict>(vm)?;
        let locals = unsafe { &*locals }.try_downcast_ref::<PyDict>(vm)?;

        let scope = Scope::with_builtins(
            Some(ArgMapping::from_dict_exact(locals.to_owned())),
            globals.to_owned(),
            vm,
        );
        vm.run_code_obj(code.to_owned(), scope)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_GetBuiltins() -> *mut PyObject {
    with_vm(|vm| vm.builtins.as_object().as_raw())
}

#[cfg(test)]
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
