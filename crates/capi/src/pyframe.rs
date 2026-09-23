use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::c_int;
use rustpython_vm::Py;
use rustpython_vm::builtins::PyCode;
use rustpython_vm::frame::FrameObject;

pub type PyFrameObject = Py<FrameObject>;
pub type PyCodeObject = Py<PyCode>;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetCode(frame: *mut PyFrameObject) -> *mut PyCodeObject {
    with_vm(|_vm| Ok(unsafe { &*frame }.f_code()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetLineNumber(frame: *mut PyFrameObject) -> c_int {
    with_vm(|_vm| Ok(unsafe { &*frame }.lineno()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetLocals(frame: *mut PyFrameObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*frame }.f_locals(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetGlobals(frame: *mut PyFrameObject) -> *mut PyObject {
    with_vm(|_vm| Ok(unsafe { &*frame }.f_globals()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetBuiltins(frame: *mut PyFrameObject) -> *mut PyObject {
    with_vm(|_vm| Ok(unsafe { &*frame }.f_builtins()))
}

#[cfg(test)]
mod tests {
    use alloc::ffi::CString;
    use pyo3::prelude::*;
    use rustpython_vm::AsObject;

    fn current_frame_ptr() -> *mut super::PyFrameObject {
        rustpython_vm::vm::thread::with_current_vm(|vm| {
            vm.current_frame()
                .unwrap()
                .as_object()
                .as_raw()
                .cast_mut()
                .cast()
        })
    }

    #[pyfunction]
    fn write_local_via_frame_get_locals(name: &str, value: i64) -> bool {
        let locals = unsafe { super::PyFrame_GetLocals(current_frame_ptr()) };
        assert!(!locals.is_null());
        let key = CString::new(name).unwrap();
        let py_value = crate::longobject::PyLong_FromLongLong(value);
        let rc =
            unsafe { crate::abstract_::PyMapping_SetItemString(locals, key.as_ptr(), py_value) };
        unsafe {
            crate::refcount::_Py_DecRef(py_value);
            crate::refcount::_Py_DecRef(locals);
        }
        rc == 0
    }

    #[pyfunction]
    fn snapshot_set_does_not_write(name: &str, value: i64) -> bool {
        let locals = crate::ceval::PyEval_GetFrameLocals();
        assert!(!locals.is_null());
        let key = CString::new(name).unwrap();
        let py_value = crate::longobject::PyLong_FromLongLong(value);
        let rc =
            unsafe { crate::abstract_::PyMapping_SetItemString(locals, key.as_ptr(), py_value) };
        unsafe {
            crate::refcount::_Py_DecRef(py_value);
            crate::refcount::_Py_DecRef(locals);
        }
        rc == 0
    }

    #[pyfunction]
    fn frame_globals_is_globals() -> bool {
        rustpython_vm::vm::thread::with_current_vm(|vm| {
            let frame = vm.current_frame().unwrap();
            let got = unsafe { super::PyFrame_GetGlobals(current_frame_ptr()) };
            let expected = frame.iframe().globals().as_object().as_raw();
            let same = core::ptr::eq(got.cast_const(), expected);
            unsafe { crate::refcount::_Py_DecRef(got) };
            same
        })
    }

    #[pyfunction]
    fn frame_builtins_is_builtins() -> bool {
        rustpython_vm::vm::thread::with_current_vm(|vm| {
            let frame = vm.current_frame().unwrap();
            let got = unsafe { super::PyFrame_GetBuiltins(current_frame_ptr()) };
            let expected = frame.iframe().builtins().as_object().as_raw();
            let same = core::ptr::eq(got.cast_const(), expected);
            unsafe { crate::refcount::_Py_DecRef(got) };
            same
        })
    }

    #[test]
    fn get_locals_globals_builtins() {
        Python::attach(|py| {
            let globals = pyo3::types::PyDict::new(py);
            globals
                .set_item(
                    "write_local_via_frame_get_locals",
                    wrap_pyfunction!(write_local_via_frame_get_locals, py).unwrap(),
                )
                .unwrap();
            globals
                .set_item(
                    "snapshot_set_does_not_write",
                    wrap_pyfunction!(snapshot_set_does_not_write, py).unwrap(),
                )
                .unwrap();
            globals
                .set_item(
                    "frame_globals_is_globals",
                    wrap_pyfunction!(frame_globals_is_globals, py).unwrap(),
                )
                .unwrap();
            globals
                .set_item(
                    "frame_builtins_is_builtins",
                    wrap_pyfunction!(frame_builtins_is_builtins, py).unwrap(),
                )
                .unwrap();
            py.run(
                c"\
def optimized():
    x = 1
    ok = write_local_via_frame_get_locals('x', 2)
    snapshot_ok = snapshot_set_does_not_write('x', 9)
    return x, ok, snapshot_ok, frame_globals_is_globals(), frame_builtins_is_builtins()

assert optimized() == (2, True, True, True, True)
",
                Some(&globals),
                None,
            )
            .unwrap();
        })
    }
}
