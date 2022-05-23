pub(crate) use _functools::make_module;

#[pymodule]
mod _functools {
    use crate::{function::OptionalArg, protocol::PyIter, PyObjectRef, PyResult, VirtualMachine};

    #[pyfunction]
    fn reduce(
        function: PyObjectRef,
        iterator: PyIter,
        start_value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let mut iter = iterator.iter_without_hint(vm)?;
        let start_value = if let OptionalArg::Present(val) = start_value {
            val
        } else {
            iter.next().transpose()?.ok_or_else(|| {
                let exc_type = vm.ctx.exceptions.type_error.to_owned();
                vm.new_exception_msg(
                    exc_type,
                    "reduce() of empty sequence with no initial value".to_owned(),
                )
            })?
        };

        let mut accumulator = start_value;
        for next_obj in iter {
            accumulator = vm.invoke(&function, (accumulator, next_obj?))?
        }
        Ok(accumulator)
    }
}
