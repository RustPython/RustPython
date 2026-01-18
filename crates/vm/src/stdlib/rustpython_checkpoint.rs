pub(crate) use rustpython_checkpoint::make_module;

#[pymodule]
mod rustpython_checkpoint {
    use crate::{PyResult, VirtualMachine, builtins::PyStrRef};
    use crate::vm::{CheckpointRequest, CheckpointTarget};

    #[pyfunction]
    fn checkpoint(path: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let frame = vm
            .current_frame()
            .ok_or_else(|| vm.new_runtime_error("checkpoint requires an active frame".to_owned()))?;
        let expected_lasti = frame.lasti();
        let mut request = vm.state.checkpoint_request.lock();
        *request = Some(crate::vm::CheckpointRequest {
            target: CheckpointTarget::File(path.as_str().to_owned()),
            expected_lasti,
        });
        Ok(())
    }

    #[pyfunction]
    fn checkpoint_bytes(vm: &VirtualMachine) -> PyResult<()> {
        let frame = vm
            .current_frame()
            .ok_or_else(|| vm.new_runtime_error("checkpoint requires an active frame".to_owned()))?;
        let expected_lasti = frame.lasti();
        let mut request = vm.state.checkpoint_request.lock();
        *request = Some(CheckpointRequest {
            target: CheckpointTarget::Bytes,
            expected_lasti,
        });
        Ok(())
    }
}
