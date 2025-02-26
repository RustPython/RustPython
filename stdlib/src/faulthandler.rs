pub(crate) use decl::make_module;

#[pymodule(name = "faulthandler")]
mod decl {
    use crate::vm::{
        frame::{Frame, FrameRef},
        function::OptionalArg,
        stdlib::sys::PyStderr,
        VirtualMachine,
    };
    use rustpython_common::lock::PyRwLock;
    use std::io::Write;

    struct FaultHandlerThread {
        enabled: bool,
        repeat: bool,
        exit: bool,
        timeout: std::time::Duration,
        frames: Vec<FrameRef>,
    }

    static FAULT_HANDLER_THREAD: PyRwLock<Option<FaultHandlerThread>> = PyRwLock::new(None);

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
        let timeout_duration = std::time::Duration::from_secs_f32(timeout);
        let repeat = repeat.unwrap_or(false);
        let exit = exit.unwrap_or(false);
        let frames = vm.frames.borrow().clone();
        let t_data = FaultHandlerThread {
            enabled: true,
            repeat,
            timeout: timeout_duration,
            frames,
            exit,
        };
        if let Some(t) = FAULT_HANDLER_THREAD.write().as_mut() {
            *t = t_data;
        } else {
            std::thread::spawn(move || {
                loop {
                    let thread_info = FAULT_HANDLER_THREAD.read();
                    let thread_info = match thread_info.as_ref() {
                        Some(t) => t,
                        None => return,
                    };
                    if !thread_info.enabled {
                        *FAULT_HANDLER_THREAD.write() = None;
                        return;
                    }

                    std::thread::sleep(thread_info.timeout);

                    let thread_info = FAULT_HANDLER_THREAD.read();
                    let thread_info = match thread_info.as_ref() {
                        Some(t) => t,
                        None => return,
                    };
                    if !thread_info.enabled {
                        *FAULT_HANDLER_THREAD.write() = None;
                        return;
                    }
                    for frame in thread_info.frames.iter() {
                        // TODO: Fix
                        let _ = writeln!(std::io::stderr(), "Stack (most recent call first):");
                        let _ = writeln!(
                            std::io::stderr(),
                            "  File \"{}\", line {} in {}",
                            frame.code.source_path,
                            frame.current_location().row.to_usize(),
                            frame.code.obj_name
                        );
                    }
                    if thread_info.exit {
                        std::process::exit(1);
                    }
                    if !thread_info.repeat {
                        *FAULT_HANDLER_THREAD.write() = None;
                        return;
                    }
                }
            });
        }
    }

    #[pyfunction]
    fn cancel_dump_traceback_later() {
        FAULT_HANDLER_THREAD
            .write()
            .as_mut()
            .map(|t| t.enabled = false);
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
