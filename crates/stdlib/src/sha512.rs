use crate::vm::{PyRef, VirtualMachine, builtins::PyModule};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let _ = vm.import("_hashlib", 0);
    _sha512::make_module(vm)
}

#[pymodule]
mod _sha512 {
    use crate::hashlib::_hashlib::{HashArgs, local_sha384, local_sha512};
    use crate::vm::{PyPayload, PyResult, VirtualMachine};

    #[pyfunction]
    fn sha384(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha384(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn sha512(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha512(args).into_pyobject(vm))
    }
}
