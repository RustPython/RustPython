use crate::PyObject;
use crate::pystate::with_vm;
use rustpython_vm::builtins::PyStr;

#[unsafe(no_mangle)]
pub extern "C" fn PyImport_Import(name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { (&*name).downcast_unchecked_ref::<PyStr>() };
        vm.import(name, 0).map_or_else(
            |err| {
                vm.push_exception(Some(err));
                std::ptr::null_mut()
            },
            |module| module.into_raw().as_ptr(),
        )
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
