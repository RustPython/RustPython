use crate::PyObject;
use crate::pystate::with_vm;
use rustpython_vm::PyPayload;
use rustpython_vm::builtins::PyMappingProxy;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDictProxy_New(mapping: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let mapping = unsafe { &*mapping }.to_owned();
        Ok(PyMappingProxy::from_object(mapping, vm)?.into_ref(&vm.ctx))
    })
}

#[cfg(false)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyInt, PyMappingProxy};

    #[test]
    fn proxy_reads_items() {
        Python::attach(|py| {
            let dict = PyDict::new(py);
            dict.set_item("x", 7).unwrap();

            let mapping = dict.as_mapping();
            let proxy = PyMappingProxy::new(py, &mapping);
            let value = proxy.get_item("x").unwrap().cast_into::<PyInt>().unwrap();
            assert_eq!(value, 7);
        })
    }
}
