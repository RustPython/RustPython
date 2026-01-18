pub(crate) use pvm_host_module::make_module;

#[rustpython_vm::pymodule]
mod pvm_host_module {
    use crate::host;
    use crate::continuation::RuntimeConfig;
    use ::pvm_host::{HostApi, HostContext, HostError};
    use rustpython_vm::{
        AsObject,
        PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyStrRef, PyTypeRef},
        function::ArgBytesLike,
    };

    fn host_error(vm: &VirtualMachine, err: HostError) -> PyBaseExceptionRef {
        let exc = vm.new_exception(
            host_error_type(vm),
            vec![vm.ctx.new_str(err.to_string()).into()],
        );
        let _ = exc
            .as_object()
            .set_attr("code", vm.new_pyobj(err.code()), vm);
        let _ = exc
            .as_object()
            .set_attr("name", vm.ctx.new_str(err.as_str()), vm);
        exc
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

    #[pyfunction]
    fn send_message(target: ArgBytesLike, payload: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
        with_host(vm, |host| {
            target.with_ref(|t| payload.with_ref(|p| host.send_message(t, p)))
        })?;
        Ok(())
    }

    #[pyfunction]
    fn schedule_timer(height: u64, payload: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let timer_id = with_host(vm, |host| {
            payload.with_ref(|p| host.schedule_timer(height, p))
        })?;
        Ok(vm.ctx.new_bytes(timer_id).into())
    }

    #[pyfunction]
    fn cancel_timer(timer_id: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
        with_host(vm, |host| timer_id.with_ref(|id| host.cancel_timer(id)))?;
        Ok(())
    }

    #[pyfunction]
    fn runtime_config(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let config = host::runtime_config();
        Ok(runtime_config_to_dict(vm, config)?.into())
    }

    #[pyattr(name = "HostError", once)]
    fn host_error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "pvm_host",
            "HostError",
            Some(vec![vm.ctx.exceptions.runtime_error.to_owned()]),
        )
    }

    #[pyattr(name = "DeterministicValidationError", once)]
    fn deterministic_validation_error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "pvm_host",
            "DeterministicValidationError",
            Some(vec![vm.ctx.exceptions.value_error.to_owned()]),
        )
    }

    #[pyattr(name = "NonDeterministicError", once)]
    fn nondeterministic_error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "pvm_host",
            "NonDeterministicError",
            Some(vec![vm.ctx.exceptions.runtime_error.to_owned()]),
        )
    }

    #[pyattr(name = "OutOfGasError", once)]
    fn out_of_gas_error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "pvm_host",
            "OutOfGasError",
            Some(vec![vm.ctx.exceptions.runtime_error.to_owned()]),
        )
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
        dict.set_item("actor_addr", vm.ctx.new_bytes(ctx.actor_addr).into(), vm)?;
        dict.set_item("msg_id", vm.ctx.new_bytes(ctx.msg_id).into(), vm)?;
        dict.set_item("nonce", vm.new_pyobj(ctx.nonce), vm)?;
        Ok(dict)
    }

    fn runtime_config_to_dict(
        vm: &VirtualMachine,
        cfg: Option<RuntimeConfig>,
    ) -> PyResult<rustpython_vm::builtins::PyDictRef> {
        let dict = vm.ctx.new_dict();
        if let Some(cfg) = cfg {
            dict.set_item(
                "continuation_mode",
                vm.ctx.new_str(cfg.continuation_mode.to_string()).into(),
                vm,
            )?;
        }
        Ok(dict)
    }

}
