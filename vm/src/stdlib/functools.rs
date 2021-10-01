pub(crate) use _functools::make_module;

#[pymodule]
mod _functools {
    use crate::function::OptionalArg;
    use crate::protocol::{PyIter, PyIterReturn};
    use crate::vm::VirtualMachine;
    use crate::{PyObjectRef, PyResult};

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
            iterator.next(vm).and_then(|iret| match iret {
                PyIterReturn::Return(obj) => Ok(obj),
                PyIterReturn::StopIteration(_) => Err({
                    let exc_type = vm.ctx.exceptions.type_error.clone();
                    vm.new_exception_msg(
                        exc_type,
                        "reduce() of empty sequence with no initial value".to_owned(),
                    )
                }),
            })?
        };

        let mut accumulator = start_value;

        while let Ok(next_obj) = iterator.next(vm) {
            let next_obj = match next_obj {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(_) => break,
            };
            accumulator = vm.invoke(&function, vec![accumulator, next_obj])?
        }

        Ok(accumulator)
    }
}
