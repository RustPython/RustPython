use crate::frame::FrameRef;
use crate::function::OptionalArg;
use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;
use std::cell::Ref;

fn dump_frame(frame: &FrameRef) {
    eprintln!(
        "  File \"{}\", line {} in {}",
        frame.code.source_path,
        frame.get_lineno().row(),
        frame.code.obj_name
    )
}

fn dump_traceback(_file: OptionalArg<i64>, _all_threads: OptionalArg<bool>, vm: &VirtualMachine) {
    eprintln!("Stack (most recent call first):");

    Ref::map(vm.frames.borrow(), |frames| {
        &for frame in frames {
            dump_frame(frame);
        }
    });
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "faulthandler", {
        "dump_traceback" => ctx.new_rustfunc(dump_traceback),
    })
}
