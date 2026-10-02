use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::util::FfiPtrExt;
use core::ffi::c_int;
use rustpython_compiler_core::OneIndexed;
use rustpython_vm::PyPayload;
use rustpython_vm::function::{FuncArgs, KwArgs};

define_py_check!(exact fn PyTraceBack_Check, types.traceback_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceBack_Print(tb: *mut PyObject, file: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let tb = unsafe { tb.assume_borrowed() };
        let file = unsafe { file.assume_borrowed() };
        let tb_module = vm.import("traceback", 0)?;
        let print_tb = tb_module.get_attr("print_tb", vm)?;

        let kwargs: KwArgs = core::iter::once(("file".to_string(), file.to_owned())).collect();
        print_tb.call(FuncArgs::new(vec![tb.to_owned()], kwargs), vm)?;

        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceBack_Here(frame: *mut crate::pyframe::PyFrameObject) -> c_int {
    with_vm(|vm| {
        let frame = unsafe { frame.assume_borrowed() };
        let Some(exc) = vm.current_exception() else {
            return Ok(());
        };
        let line = frame.lineno().max(1) as usize;
        let lineno = OneIndexed::new(line).unwrap();
        let lasti = frame.lasti() as i32;
        let old_tb = exc.traceback();
        let new_tb =
            rustpython_vm::builtins::PyTraceback::new(old_tb, frame.to_owned(), lasti, lineno)
                .into_ref(&vm.ctx);
        exc.set_traceback(Some(new_tb));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;

    #[pyfunction]
    fn trigger_traceback_here() -> bool {
        let frame = crate::ceval::PyEval_GetFrame();
        if frame.is_null() {
            return false;
        }
        unsafe { super::PyTraceBack_Here(frame) == 0 }
    }

    #[test]
    fn traceback_here_and_check() {
        Python::attach(|py| {
            let func = pyo3::wrap_pyfunction!(trigger_traceback_here, py).unwrap();
            let globals = pyo3::types::PyDict::new(py);
            globals.set_item("trigger_tb", func).unwrap();
            py.run(
                c"
def run():
    return trigger_tb()
assert run()
",
                Some(&globals),
                None,
            )
            .unwrap();

            let c_msg = alloc::ffi::CString::new("test err").unwrap();
            unsafe {
                crate::pyerrors::PyErr_SetString(crate::pyerrors::PyExc_ValueError, c_msg.as_ptr());
                let occ = crate::pyerrors::PyErr_Occurred();
                assert!(!occ.is_null());
                let raised = crate::pyerrors::PyErr_GetRaisedException();
                assert!(!raised.is_null());
                crate::refcount::_Py_DecRef(raised);

                let not_tb = crate::longobject::PyLong_FromLongLong(42);
                assert_eq!(super::PyTraceBack_Check(not_tb), 0);
                crate::refcount::_Py_DecRef(not_tb);
            }
        });
    }
}
