pub(crate) use decl::make_module;

#[pymodule(name = "faulthandler")]
mod decl {
    use std::sync::atomic::AtomicBool;
    use rustpython_common::lock::{PyMutex, PyRwLock};
    use crate::vm::{frame::Frame, function::OptionalArg, stdlib::sys::PyStderr, VirtualMachine};

    struct FaultHandlerThread {
        enabled: AtomicBool,
        name: PyMutex<String>
    }

    static FAULT_HANDLER_THREADS: PyRwLock<Vec<FaultHandlerThread>> = PyRwLock::new(Vec::new());

    fn dump_frame(frame: &Frame, vm: &VirtualMachine) {
        let stderr = PyStderr(vm);
        writeln!(
            stderr,
            "  File \"{}\", line {} in {}",
            frame.code.source_path,
            frame.current_location().row.to_usize(),
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

    #[pyfunction]
    fn dump_traceback_later(
        timeout: f32,
        repeat: OptionalArg<bool>,
        _file: OptionalArg<i32>,
        exit: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) {
        let timeout_micros = std::time::Duration::from_secs_f32(timeout);
        let repeat = repeat.unwrap_or(false);
        let exit = exit.unwrap_or(false);

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
