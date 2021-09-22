pub(crate) use _functools::make_module;

#[pymodule]
mod _functools {
    use crate::function::OptionalArg;
    use crate::protocol::PyIter;
    use crate::vm::VirtualMachine;
    use crate::{PyObjectRef, PyResult, TypeProtocol};

    #[pyfunction]
    fn reduce(
        function: PyObjectRef,
        iterator: PyIter,
        start_value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let start_value = if let OptionalArg::Present(val) = start_value {
            val
        } else {
            iterator.next(vm).map_err(|err| {
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

        while let Ok(next_obj) = iterator.next(vm) {
            accumulator = vm.invoke(&function, vec![accumulator, next_obj])?
        }

        Ok(accumulator)
    }
}
