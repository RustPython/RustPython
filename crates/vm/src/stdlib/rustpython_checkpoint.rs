pub(crate) use rustpython_checkpoint::make_module;

#[pymodule]
mod rustpython_checkpoint {
    use crate::{PyResult, VirtualMachine, builtins::PyStrRef};

    #[pyfunction]
    fn checkpoint(path: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let frame = vm
            .current_frame()
            .ok_or_else(|| vm.new_runtime_error("checkpoint requires an active frame".to_owned()))?;
        let expected_lasti = frame.lasti();
        let mut request = vm.state.checkpoint_request.lock();
        *request = Some(crate::vm::CheckpointRequest {
            path: path.as_str().to_owned(),
            expected_lasti,
        });
        Ok(())
    }
}
