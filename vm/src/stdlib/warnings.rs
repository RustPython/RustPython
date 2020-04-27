use crate::function::OptionalArg;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::{self, PyClassRef};
use crate::pyobject::{PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

#[derive(FromArgs)]
struct WarnArgs {
    #[pyarg(positional_only, optional = false)]
    message: PyStringRef,
    #[pyarg(positional_or_keyword, optional = true)]
    category: OptionalArg<PyClassRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    stacklevel: OptionalArg<u32>,
}

fn warnings_warn(args: WarnArgs, vm: &VirtualMachine) -> PyResult<()> {
    // TODO: Implement correctly
    let level = args.stacklevel.unwrap_or(1);
    let category = if let OptionalArg::Present(category) = args.category {
        if !objtype::issubclass(&category, &vm.ctx.exceptions.warning) {
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "_warnings", {
         "warn" => ctx.new_function(warnings_warn),
    })
}
