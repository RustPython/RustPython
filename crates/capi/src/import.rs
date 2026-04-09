use crate::{PyObject, with_vm};
use rustpython_vm::builtins::PyStr;

#[unsafe(no_mangle)]
pub extern "C" fn PyImport_Import(name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { (&*name).try_downcast_ref::<PyStr>(vm)? };
        vm.import(name, 0)
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;

    #[test]
    fn test_import() {
        Python::attach(|py| {
            let _module = py.import("sys").unwrap();
        })
    }
}
