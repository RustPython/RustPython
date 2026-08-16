pub use atexit::_run_exitfuncs;
pub(crate) use atexit::module_def;

#[pymodule]
mod atexit {
    use crate::{
        AsObject, PyObjectRef, PyResult, VirtualMachine, common::rc::PyRc, function::FuncArgs,
    };

    #[pyfunction]
    fn register(func: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
        // Callbacks go in LIFO order (insert at front)
        vm.state
            .atexit_funcs
            .lock()
            .insert(0, PyRc::new((func.clone(), args)));
        func
    }

    #[pyfunction]
    fn _clear(vm: &VirtualMachine) {
        vm.state.atexit_funcs.lock().clear();
    }

    #[pyfunction]
    fn unregister(func: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Iterate backward (oldest to newest in LIFO list).
        // Release the lock during comparison so __eq__ can call atexit functions.
        let mut i = {
            let funcs = vm.state.atexit_funcs.lock();
            funcs.len() as isize - 1
        };
        while i >= 0 {
            let entry = {
                let funcs = vm.state.atexit_funcs.lock();
                if i as usize >= funcs.len() {
                    i = funcs.len() as isize;
                    i -= 1;
                    continue;
                }
                // Keep the entry alive for as long as it is being compared, so
                // it cannot be dropped and have its address handed to a
                // callback registered from within __eq__.
                funcs[i as usize].clone()
            };
            // Lock released: __eq__ can safely call atexit functions
            let eq = vm.bool_eq(&func, &entry.0)?;
            if eq {
                // The entry may have moved during __eq__. Search backward by identity.
                let mut funcs = vm.state.atexit_funcs.lock();
                let mut j = (funcs.len() as isize - 1).min(i);
                while j >= 0 {
                    if PyRc::ptr_eq(funcs.get(j as usize).unwrap(), &entry) {
                        funcs.remove(j as usize);
                        i = j;
                        break;
                    }
                    j -= 1;
                }
            }
            {
                let funcs = vm.state.atexit_funcs.lock();
                if i as usize >= funcs.len() {
                    i = funcs.len() as isize;
                }
            }
            i -= 1;
        }
        Ok(())
    }

    #[pyfunction]
    pub fn _run_exitfuncs(vm: &VirtualMachine) {
        let funcs: Vec<_> = core::mem::take(&mut *vm.state.atexit_funcs.lock());
        // Callbacks stored in LIFO order, iterate forward
        for entry in funcs {
            let (func, args) = PyRc::try_unwrap(entry).unwrap_or_else(|e| (*e).clone());
            if let Err(e) = func.call(args, vm) {
                let exit = e.fast_isinstance(vm.ctx.exceptions.system_exit);
                let msg = func
                    .repr(vm)
                    .ok()
                    .map(|r| format!("Exception ignored in atexit callback {}", r.as_wtf8()));
                vm.run_unraisable(e, msg, vm.ctx.none());
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
