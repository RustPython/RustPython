pub(crate) use _functools::make_module;

#[pymodule]
mod _functools {
    use crate::function::OptionalArg;
    use crate::iterator;
    use crate::pyobject::{PyObjectRef, PyResult, TypeProtocol};
    use crate::vm::VirtualMachine;

    #[pyfunction]
    fn reduce(
        function: PyObjectRef,
        sequence: PyObjectRef,
        start_value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let iterator = iterator::get_iter(vm, sequence)?;

        let start_value = if let OptionalArg::Present(val) = start_value {
            val
        } else {
            iterator::call_next(vm, &iterator).map_err(|err| {
                if err.isinstance(&vm.ctx.exceptions.stop_iteration) {
                    let exc_type = vm.ctx.exceptions.type_error.clone();
                    vm.new_exception_msg(
                        exc_type,
                        "reduce() of empty sequence with no initial value".to_owned(),
                    )
                } else {
                    err
                }
            })?
        };

        let mut accumulator = start_value;

        while let Ok(next_obj) = iterator::call_next(vm, &iterator) {
            accumulator = vm.invoke(&function, vec![accumulator, next_obj])?
        }

        Ok(accumulator)
    }
}
