pub(crate) use _warnings::make_module;

#[pymodule]
mod _warnings {
    use crate::function::OptionalArg;
    use crate::obj::objstr::PyStrRef;
    use crate::obj::objtype::{self, PyTypeRef};
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
            if !objtype::issubclass(&category, &vm.ctx.exceptions.warning) {
                return Err(vm.new_type_error(format!(
                    "category must be a Warning subclass, not '{}'",
                    category.lease_class().name
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
