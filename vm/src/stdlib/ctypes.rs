use crate::pyobject::PyObjectRef;
use crate::VirtualMachine;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = _ctypes::make_module(vm);
    module
}

#[pymodule]
mod _ctypes {
    extern crate libloading;
    use crate::builtins::pystr::PyStrRef;
    use crate::builtins::pytype::PyTypeRef;
    use crate::pyobject::{PyObjectRef, PyResult, PyValue};
    use crate::VirtualMachine;

    // #[pyclass()]
    #[derive(Debug)]
    struct SharedLibrary {
        lib: libloading::Library,
    }

    impl PyValue for SharedLibrary {
        fn class(vm: &VirtualMachine) -> &PyTypeRef {
            &vm.ctx.types.object_type
        }
    }

    #[pyfunction]
    fn dlopen(lib_path: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let shared_lib = SharedLibrary {
            lib: libloading::Library::new(lib_path.as_ref()).expect("Failed to load library"),
        };
        Ok(vm.new_pyobj(shared_lib))
    }

    #[pyfunction]
    fn dlsym(handle: PyObjectRef, func_name: PyStrRef) {
        if let Some(slib) = handle.payload::<SharedLibrary>() {
            unsafe {
                let func: libloading::Symbol<unsafe extern "C" fn()> = slib
                    .lib
                    .get(func_name.as_ref().as_bytes())
                    .expect("Failed to get func");
                func();
            }
        }
    }
}
