// spell-checker:ignore usedforsecurity HASHXOF

pub(crate) use _blake2::make_module;

#[pymodule]
mod _blake2 {
    use crate::hashlib::_hashlib::{BlakeHashArgs, local_blake2b, local_blake2s};
    use crate::vm::{PyPayload, PyResult, VirtualMachine};

    #[pyfunction]
    fn blake2b(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_blake2b(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn blake2s(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_blake2s(args).into_pyobject(vm))
    }
}
