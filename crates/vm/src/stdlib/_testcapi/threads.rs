use crate::{
    Py, PyObjectRef, PyResult, VirtualMachine,
    builtins::PyModule,
    function::{ItemDoc, OptionalArg, PyMethodDef, PyMethodFlags},
    vm::thread::{attach_current_thread, release_current_thread, with_current_vm},
};
use std::sync::{Mutex, mpsc};
use std::thread::JoinHandle;

struct TemporaryThread {
    interpreter_id: i64,
    handle: JoinHandle<()>,
}

static TEMPORARY_THREAD: Mutex<Option<TemporaryThread>> = Mutex::new(None);

fn join_temporary_c_thread(vm: &VirtualMachine) -> PyResult<()> {
    let thread = {
        let mut pending = vm.allow_threads(|| {
            TEMPORARY_THREAD
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        });
        let Some(thread) = pending.as_ref() else {
            return Err(vm.new_runtime_error("no temporary thread is running"));
        };
        if thread.interpreter_id != vm.state.interpreter_id {
            return Err(vm.new_runtime_error("temporary thread belongs to another interpreter"));
        }
        if thread.handle.thread().id() == std::thread::current().id() {
            return Err(vm.new_runtime_error("cannot join the current thread"));
        }
        pending.take().unwrap()
    };
    vm.allow_threads(|| thread.handle.join())
        .map_err(|_| vm.new_runtime_error("temporary thread panicked"))
}

fn call_in_temporary_c_thread(
    callback: PyObjectRef,
    wait: OptionalArg<i32>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if !vm.state.allow_threads() {
        return Err(vm.new_runtime_error("thread is not supported for isolated subinterpreters"));
    }
    if vm
        .state
        .finalizing
        .load(core::sync::atomic::Ordering::Acquire)
    {
        return Err(vm.new_exception_msg(
            vm.ctx.exceptions.python_finalization_error.to_owned(),
            "can't create new thread at interpreter shutdown".into(),
        ));
    }
    let (started, ready) = mpsc::sync_channel(1);
    {
        let mut pending = vm.allow_threads(|| {
            TEMPORARY_THREAD
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        });
        if pending.is_some() {
            return Err(vm.new_runtime_error("temporary thread already running"));
        }
        let thread_vm = vm.new_thread();
        let builder =
            crate::stdlib::_thread::apply_thread_stack_size(std::thread::Builder::new(), vm);
        let handle = builder
            .spawn(move || {
                // Match the foreign-thread fixture: signal startup before attaching
                // a Python thread state, then release that state after the callback.
                let _ = started.send(());
                let state = attach_current_thread(|| thread_vm);
                scopeguard::defer! { release_current_thread(state); }
                with_current_vm(|vm| {
                    let callback = callback;
                    if let Err(exception) = callback.call((), vm) {
                        vm.print_exception(&exception);
                    }
                });
            })
            .map_err(|_| vm.new_runtime_error("unable to start the thread"))?;
        *pending = Some(TemporaryThread {
            interpreter_id: vm.state.interpreter_id,
            handle,
        });
    }
    vm.allow_threads(|| ready.recv())
        .map_err(|_| vm.new_runtime_error("temporary thread failed to start"))?;
    if wait.unwrap_or(1) != 0 {
        join_temporary_c_thread(vm)?;
    }
    Ok(())
}

pub(super) fn extend_module(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
    const METHODS: &[PyMethodDef] = &[
        PyMethodDef::new_const(
            "call_in_temporary_c_thread",
            call_in_temporary_c_thread,
            PyMethodFlags::VARARGS,
            ItemDoc::NONE,
        ),
        PyMethodDef::new_const(
            "join_temporary_c_thread",
            join_temporary_c_thread,
            PyMethodFlags::NOARGS,
            ItemDoc::NONE,
        ),
    ];
    for method in METHODS {
        let function = method.build_function(&vm.ctx);
        drop(
            function
                .module
                .store(Some(vm.ctx.new_str("_testcapi").into())),
        );
        module.set_attr(method.name, function, vm)?;
    }
    Ok(())
}
