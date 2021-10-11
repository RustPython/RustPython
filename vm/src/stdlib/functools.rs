pub(crate) use _functools::make_module;

#[pymodule]
mod _functools {
    use crate::function::OptionalArg;
    use crate::protocol::PyIter;
    use crate::vm::VirtualMachine;
    use crate::{PyObjectRef, PyResult};

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
                let exc_type = vm.ctx.exceptions.type_error.clone();
                vm.new_exception_msg(
                    exc_type,
                    "reduce() of empty sequence with no initial value".to_owned(),
                )
            })?
        };

        let mut accumulator = start_value;
        for next_obj in iter {
            accumulator = vm.invoke(&function, vec![accumulator, next_obj?])?
        }
        Ok(accumulator)
    }
}
