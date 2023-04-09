// spell-checker:ignore usedforsecurity HASHXOF

pub(crate) use _blake2::make_module;

#[pymodule]
mod _blake2 {
    use crate::hashlib::_hashlib::{BlakeHashArgs, HashWrapper, PyHasher};
    use crate::vm::{PyObjectRef, PyPayload, PyResult, VirtualMachine};
    use blake2::{Blake2b512, Blake2s256};

    #[pyfunction(name = "blake2b")]
    fn blake2b(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("blake2b", HashWrapper::new::<Blake2b512>(args.data)).into_pyobject(vm))
    }

    #[pyfunction(name = "blake2s")]
    fn blake2s(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("blake2s", HashWrapper::new::<Blake2s256>(args.data)).into_pyobject(vm))
    }
}
