use crate::vm::{PyRef, VirtualMachine, builtins::PyModule};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let _ = vm.import("_hashlib", 0);
    _sha256::make_module(vm)
}

#[pymodule]
mod _sha256 {
    use crate::hashlib::_hashlib::{HashArgs, local_sha224, local_sha256};
    use crate::vm::{PyPayload, PyResult, VirtualMachine};

    #[pyfunction]
    fn sha224(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha224(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn sha256(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha256(args).into_pyobject(vm))
    }
}
