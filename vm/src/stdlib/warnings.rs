pub(crate) use _warnings::make_module;

use crate::{builtins::PyTypeRef, PyResult, VirtualMachine};

pub fn warn(
    category: PyTypeRef,
    message: String,
    stack_level: usize,
    vm: &VirtualMachine,
) -> PyResult<()> {
    // let module = vm.import("warnings", None, 0)?;
    // let func = module.get_attr("warn", vm)?;
    // vm.invoke(&func, (message, category, stack_level))?;
    // TODO
    if let Ok(module) = vm.import("warnings", None, 0) {
        if let Ok(func) = module.get_attr("warn", vm) {
            let _ = vm.invoke(&func, (message, category, stack_level));
        }
    }
    Ok(())
}

#[pymodule]
mod _warnings {
    use crate::{
        builtins::{PyStrRef, PyTypeRef},
        function::OptionalArg,
        stdlib::sys::PyStderr,
        AsObject, PyResult, VirtualMachine,
    };

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
            if !category.fast_issubclass(vm.ctx.exceptions.warning) {
                return Err(vm.new_type_error(format!(
                    "category must be a Warning subclass, not '{}'",
                    category.class().name()
                )));
            }
            category
        } else {
            vm.ctx.exceptions.user_warning.to_owned()
        };
        let stderr = PyStderr(vm);
        writeln!(
            stderr,
            "level:{}: {}: {}",
            level,
            category.name(),
            args.message
        );
        Ok(())
    }
}
