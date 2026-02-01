pub(crate) use _warnings::module_def;

use crate::{Py, PyResult, VirtualMachine, builtins::PyType};

pub fn warn(
    category: &Py<PyType>,
    message: String,
    stack_level: usize,
    vm: &VirtualMachine,
) -> PyResult<()> {
    // TODO: use rust warnings module
    if let Ok(module) = vm.import("warnings", 0)
        && let Ok(func) = module.get_attr("warn", vm)
    {
        func.call((message, category.to_owned(), stack_level), vm)?;
    }
    Ok(())
}

#[pymodule]
mod _warnings {
    use crate::{
        PyResult, VirtualMachine,
        builtins::{PyStrRef, PyTypeRef},
        function::OptionalArg,
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
        let level = args.stacklevel.unwrap_or(1);
        crate::warn::warn(
            args.message,
            args.category.into_option(),
            level as isize,
            None,
            vm,
        )
    }
}
