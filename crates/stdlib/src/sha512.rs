#[pymodule]
mod _sha512 {
    use crate::hashlib::_hashlib::{HashArgs, local_sha384, local_sha512};
    use crate::vm::{Py, PyPayload, PyResult, VirtualMachine, builtins::PyModule};

    #[pyfunction]
    fn sha384(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha384(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn sha512(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha512(args).into_pyobject(vm))
    }

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        let _ = vm.import("_hashlib", 0);
        __module_exec(vm, module);
        Ok(())
    }
}

pub(crate) use _sha512::module_def;
