// spell-checker:ignore usedforsecurity HASHXOF

pub(crate) use _sha256::make_module;

#[pymodule]
mod _sha256 {
    use crate::hashlib::_hashlib::{HashArgs, HashWrapper, PyHasher};
    use crate::vm::{PyObjectRef, PyPayload, PyResult, VirtualMachine};
    use sha2::{Sha224, Sha256};

    #[pyfunction(name = "sha224")]
    fn sha224(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("sha224", HashWrapper::new::<Sha224>(args.string)).into_pyobject(vm))
    }

    #[pyfunction(name = "sha256")]
    fn sha256(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("sha256", HashWrapper::new::<Sha256>(args.string)).into_pyobject(vm))
    }
}
