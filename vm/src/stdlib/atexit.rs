pub use atexit::_run_exitfuncs;
pub(crate) use atexit::make_module;

#[pymodule]
mod atexit {
    use crate::{function::FuncArgs, PyObjectRef, PyResult, TypeProtocol, VirtualMachine};

    #[pyfunction]
    fn register(func: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
        vm.state.atexit_funcs.lock().push((func.clone(), args));
        func
    }

    #[pyfunction]
    fn _clear(vm: &VirtualMachine) {
        vm.state.atexit_funcs.lock().clear();
    }

    #[pyfunction]
    fn unregister(func: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut funcs = vm.state.atexit_funcs.lock();

        let mut i = 0;
        while i < funcs.len() {
            if vm.bool_eq(&funcs[i].0, &func)? {
                funcs.remove(i);
            } else {
                i += 1;
            }
        }

        Ok(())
    }

    #[pyfunction]
    pub fn _run_exitfuncs(vm: &VirtualMachine) -> PyResult<()> {
        let mut last_exc = None;
        for (func, args) in vm.state.atexit_funcs.lock().drain(..).rev() {
            if let Err(e) = vm.invoke(&func, args) {
                last_exc = Some(e.clone());
                if !e.isinstance(&vm.ctx.exceptions.system_exit) {
                    writeln!(
                        crate::stdlib::sys::PyStderr(vm),
                        "Error in atexit._run_exitfuncs:"
                    );
                    vm.print_exception(e);
                }
            }
        }
        match last_exc {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }

    #[pyfunction]
    fn _ncallbacks(vm: &VirtualMachine) -> usize {
        vm.state.atexit_funcs.lock().len()
    }
}
