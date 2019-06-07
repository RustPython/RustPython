use crate::function::OptionalArg;
use crate::obj::objstr::PyStringRef;
use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;

#[derive(FromArgs)]
struct WarnArgs {
    #[pyarg(positional_only, optional = false)]
    message: PyStringRef,
    #[pyarg(positional_or_keyword, optional = true)]
    category: OptionalArg<PyObjectRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    stacklevel: OptionalArg<u32>,
}

fn warnings_warn(args: WarnArgs, _vm: &VirtualMachine) {
    // TODO: Implement correctly
    let level = match args.stacklevel {
        OptionalArg::Present(l) => l,
        OptionalArg::Missing => 1,
    };
    eprintln!(
        "Warning: {} , category: {:?}, level: {}",
        args.message.as_str(),
        args.category,
        level
    )
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    let module = py_module!(vm, "_warnings", {
         "warn" => ctx.new_rustfunc(warnings_warn),
    });

    module
}
