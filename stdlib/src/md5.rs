// spell-checker:ignore usedforsecurity HASHXOF

pub(crate) use _md5::make_module;

#[pymodule]
mod _md5 {
    use crate::hashlib::_hashlib::{HashArgs, HashWrapper, PyHasher};
    use crate::vm::{PyObjectRef, PyPayload, PyResult, VirtualMachine};
    use md5::Md5;

    #[pyfunction(name = "md5")]
    fn md5(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("md5", HashWrapper::new::<Md5>(args.string)).into_pyobject(vm))
    }
}
