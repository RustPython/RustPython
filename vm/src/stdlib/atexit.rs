pub(crate) use atexit::make_module;

#[pymodule]
mod atexit {
    use crate::function::FuncArgs;
    use crate::pyobject::{PyObjectRef, PyResult};
    use crate::VirtualMachine;

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
    fn _run_exitfuncs(vm: &VirtualMachine) -> PyResult<()> {
        vm.run_atexit_funcs()
    }

    #[pyfunction]
    fn _ncallbacks(vm: &VirtualMachine) -> usize {
        vm.state.atexit_funcs.lock().len()
    }
}
