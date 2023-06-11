pub(crate) use _warnings::make_module;

use crate::{builtins::PyType, Py, PyResult, VirtualMachine};

pub fn warn(
    category: &Py<PyType>,
    message: String,
    stack_level: usize,
    vm: &VirtualMachine,
) -> PyResult<()> {
    // TODO: use rust warnings module
    if let Ok(module) = vm.import("warnings", None, 0) {
        if let Ok(func) = module.get_attr("warn", vm) {
            let _ = func.call((message, category.to_owned(), stack_level), vm);
        }
    }
    Ok(())
}

#[pymodule]
mod _warnings {
    use crate::{
        builtins::{PyStrRef, PyTypeRef},
        function::OptionalArg,
        PyResult, VirtualMachine,
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
