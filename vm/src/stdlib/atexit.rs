pub use atexit::_run_exitfuncs;
pub(crate) use atexit::make_module;

#[pymodule]
mod atexit {
    use crate::{function::FuncArgs, AsObject, PyObjectRef, PyResult, VirtualMachine};

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
    pub fn _run_exitfuncs(vm: &VirtualMachine) {
        let funcs: Vec<_> = std::mem::take(&mut *vm.state.atexit_funcs.lock());
        for (func, args) in funcs.into_iter().rev() {
            if let Err(e) = vm.invoke(&func, args) {
                let exit = e.fast_isinstance(&vm.ctx.exceptions.system_exit);
                vm.run_unraisable(e, Some("Error in atexit._run_exitfuncs".to_owned()), func);
                if exit {
                    break;
                }
            }
        }
    }

    #[pyfunction]
    fn _ncallbacks(vm: &VirtualMachine) -> usize {
        vm.state.atexit_funcs.lock().len()
    }
}
