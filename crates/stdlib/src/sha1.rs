pub(crate) use _sha1::make_module;

#[pymodule]
mod _sha1 {
    use crate::hashlib::_hashlib::{HashArgs, local_sha1};
    use crate::vm::{PyPayload, PyResult, VirtualMachine};

    #[pyfunction]
    fn sha1(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha1(args).into_pyobject(vm))
    }
}
