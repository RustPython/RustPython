use crate::frame::FrameRef;
use crate::function::OptionalArg;
use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;

fn dump_frame(frame: &FrameRef) {
    eprintln!(
        "  File \"{}\", line {} in {}",
        frame.code.source_path,
        frame.current_location().row(),
        frame.code.obj_name
    )
}

fn dump_traceback(_file: OptionalArg<i64>, _all_threads: OptionalArg<bool>, vm: &VirtualMachine) {
    eprintln!("Stack (most recent call first):");

    for frame in vm.frames.borrow().iter() {
        dump_frame(frame);
    }
}

#[derive(FromArgs)]
#[allow(unused)]
struct EnableArgs {
    #[pyarg(positional_or_keyword, default = "None")]
    file: Option<i64>,
    #[pyarg(positional_or_keyword, default = "true")]
    all_threads: bool,
}

fn enable(_args: EnableArgs) {
    // TODO
}

#[derive(FromArgs)]
#[allow(unused)]
struct RegisterArgs {
    #[pyarg(positional_only)]
    signum: i64,
    #[pyarg(positional_or_keyword, default = "None")]
    file: Option<i64>,
    #[pyarg(positional_or_keyword, default = "true")]
    all_threads: bool,
    #[pyarg(positional_or_keyword, default = "false")]
    chain: bool,
}

fn register(_args: RegisterArgs) {
    // TODO
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "faulthandler", {
        "dump_traceback" => ctx.new_function(dump_traceback),
        "enable" => ctx.new_function(enable),
        "register" => ctx.new_function(register),
    })
}
