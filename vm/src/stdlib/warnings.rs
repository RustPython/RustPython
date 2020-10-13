pub(crate) use _warnings::make_module;

#[pymodule]
mod _warnings {
    use crate::builtins::pystr::PyStrRef;
    use crate::builtins::pytype::PyTypeRef;
    use crate::function::OptionalArg;
    use crate::pyobject::{PyResult, TypeProtocol};
    use crate::vm::VirtualMachine;

    #[derive(FromArgs)]
    struct WarnArgs {
        #[pyarg(positional)]
        message: PyStrRef,
        #[pyarg(any, optional)]
        category: OptionalArg<PyTypeRef>,
        #[pyarg(any, optional)]
        stacklevel: OptionalArg<u32>,
    }

    #[pyfunction]
    fn warn(args: WarnArgs, vm: &VirtualMachine) -> PyResult<()> {
        // TODO: Implement correctly
        let level = args.stacklevel.unwrap_or(1);
        let category = if let OptionalArg::Present(category) = args.category {
            if !category.issubclass(&vm.ctx.exceptions.warning) {
                return Err(vm.new_type_error(format!(
                    "category must be a Warning subclass, not '{}'",
                    category.class().name
                )));
            }
            category
        } else {
            vm.ctx.exceptions.user_warning.clone()
        };
        eprintln!("level:{}: {}: {}", level, category.name, args.message);
        Ok(())
    }
}
