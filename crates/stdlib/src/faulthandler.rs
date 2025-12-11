pub(crate) use decl::make_module;

#[pymodule(name = "faulthandler")]
mod decl {
    use crate::vm::{VirtualMachine, frame::Frame, function::OptionalArg, stdlib::sys::PyStderr};
    use std::sync::atomic::{AtomicBool, Ordering};

    static ENABLED: AtomicBool = AtomicBool::new(false);

    fn dump_frame(frame: &Frame, vm: &VirtualMachine) {
        let stderr = PyStderr(vm);
        writeln!(
            stderr,
            "  File \"{}\", line {} in {}",
            frame.code.source_path,
            frame.current_location().line,
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
        #[pyarg(any, default = true)]
        all_threads: bool,
    }

    #[pyfunction]
    fn enable(_args: EnableArgs) {
        ENABLED.store(true, Ordering::Relaxed);
    }

    #[pyfunction]
    fn disable() {
        ENABLED.store(false, Ordering::Relaxed);
    }

    #[pyfunction]
    fn is_enabled() -> bool {
        ENABLED.load(Ordering::Relaxed)
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct RegisterArgs {
        #[pyarg(positional)]
        signum: i64,
        #[pyarg(any, default)]
        file: Option<i64>,
        #[pyarg(any, default = true)]
        all_threads: bool,
        #[pyarg(any, default = false)]
        chain: bool,
    }

    #[pyfunction]
    const fn register(_args: RegisterArgs) {
        // TODO
    }
}
