pub(crate) use decl::make_module;

#[pymodule(name = "faulthandler")]
mod decl {
    use crate::vm::{
        frame::FrameRef, function::OptionalArg, stdlib::sys::PyStderr, VirtualMachine,
    };

    fn dump_frame(frame: &FrameRef, vm: &VirtualMachine) {
        let stderr = PyStderr(vm);
        writeln!(
            stderr,
            "  File \"{}\", line {} in {}",
            frame.code.source_path,
            frame.current_location().row(),
            frame.code.obj_name
        )
    }

    #[pyfunction]
    fn dump_traceback(
        _file: OptionalArg<i64>,
        _all_threads: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) {
        let stderr = PyStderr(vm);
        writeln!(stderr, "Stack (most recent call first):");

        for frame in vm.frames.borrow().iter() {
            dump_frame(frame, vm);
        }
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct EnableArgs {
        #[pyarg(any, default)]
        file: Option<i64>,
        #[pyarg(any, default = "true")]
        all_threads: bool,
    }

    #[pyfunction]
    fn enable(_args: EnableArgs) {
        // TODO
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct RegisterArgs {
        #[pyarg(positional)]
        signum: i64,
        #[pyarg(any, default)]
        file: Option<i64>,
        #[pyarg(any, default = "true")]
        all_threads: bool,
        #[pyarg(any, default = "false")]
        chain: bool,
    }

    #[pyfunction]
    fn register(_args: RegisterArgs) {
        // TODO
    }
}
