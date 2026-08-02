use crate::object::define_py_check;
use crate::{PyObject, pystate::with_vm};
use rustpython_vm::PyPayload;
use rustpython_vm::builtins::PyMemoryView;

define_py_check!(fn PyMemoryView_Check, types.memoryview_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMemoryView_FromObject(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        Ok(PyMemoryView::from_object(obj, vm)?.into_ref(&vm.ctx))
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyBytes, PyMemoryView};

    #[test]
    fn memoryview_from_bytes() {
        Python::attach(|py| {
            let bytes = PyBytes::new(py, b"hello");
            let view = PyMemoryView::from(&bytes).unwrap();

            assert!(view.is_instance_of::<PyMemoryView>());

            let copied = view
                .call_method1("tobytes", ())
                .unwrap()
                .cast_into::<PyBytes>()
                .unwrap();
            assert_eq!(copied.as_bytes(), b"hello");
        })
    }
}
