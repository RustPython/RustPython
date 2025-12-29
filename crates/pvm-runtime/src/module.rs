pub(crate) use pvm_host_module::make_module;

#[rustpython_vm::pymodule]
mod pvm_host_module {
    use crate::host;
    use ::pvm_host::{HostApi, HostContext, HostError};
    use rustpython_vm::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyStrRef},
        function::ArgBytesLike,
    };

    fn host_error(vm: &VirtualMachine, err: HostError) -> PyBaseExceptionRef {
        vm.new_runtime_error(format!("pvm host error: {err}"))
    }

    fn with_host<R>(
        vm: &VirtualMachine,
        f: impl FnOnce(&mut dyn HostApi) -> Result<R, HostError>,
    ) -> PyResult<R> {
        let result = host::with_host(f)
            .ok_or_else(|| vm.new_runtime_error("pvm host is not initialized".to_owned()))?;
        result.map_err(|err| host_error(vm, err))
    }

    #[pyfunction]
    fn get_state(key: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let value = with_host(vm, |host| key.with_ref(|bytes| host.state_get(bytes)))?;
        Ok(match value {
            Some(data) => vm.ctx.new_bytes(data).into(),
            None => vm.ctx.none(),
        })
    }

    #[pyfunction]
    fn set_state(key: ArgBytesLike, value: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
        with_host(vm, |host| {
            key.with_ref(|k| value.with_ref(|v| host.state_set(k, v)))
        })?;
        Ok(())
    }

    #[pyfunction]
    fn delete_state(key: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
        with_host(vm, |host| key.with_ref(|bytes| host.state_delete(bytes)))?;
        Ok(())
    }

    #[pyfunction]
    fn emit_event(topic: PyStrRef, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
        with_host(vm, |host| data.with_ref(|bytes| host.emit_event(topic.as_str(), bytes)))?;
        Ok(())
    }

    #[pyfunction]
    fn charge_gas(amount: u64, vm: &VirtualMachine) -> PyResult<()> {
        with_host(vm, |host| host.charge_gas(amount))?;
        Ok(())
    }

    #[pyfunction]
    fn gas_left(vm: &VirtualMachine) -> PyResult<u64> {
        with_host(vm, |host| Ok(host.gas_left()))
    }

    #[pyfunction]
    fn context(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let ctx = with_host(vm, |host| Ok(host.context()))?;
        Ok(host_context_to_dict(vm, ctx)?.into())
    }

    #[pyfunction]
    fn randomness(domain: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let bytes = with_host(vm, |host| domain.with_ref(|d| host.randomness(d)))?;
        Ok(vm.ctx.new_bytes(bytes.to_vec()).into())
    }

    fn host_context_to_dict(
        vm: &VirtualMachine,
        ctx: HostContext,
    ) -> PyResult<rustpython_vm::builtins::PyDictRef> {
        let dict = vm.ctx.new_dict();
        dict.set_item("block_height", vm.new_pyobj(ctx.block_height), vm)?;
        dict.set_item(
            "block_hash",
            vm.ctx.new_bytes(ctx.block_hash.to_vec()).into(),
            vm,
        )?;
        dict.set_item(
            "tx_hash",
            vm.ctx.new_bytes(ctx.tx_hash.to_vec()).into(),
            vm,
        )?;
        dict.set_item("sender", vm.ctx.new_bytes(ctx.sender).into(), vm)?;
        dict.set_item("timestamp_ms", vm.new_pyobj(ctx.timestamp_ms), vm)?;
        Ok(dict)
    }

}
