pub(crate) use gc::make_module;

#[pymodule]
mod gc {
    use crate::vm::{function::FuncArgs, PyResult, VirtualMachine};

    #[pyfunction]
    fn collect(_args: FuncArgs, _vm: &VirtualMachine) -> i32 {
        #[cfg(feature = "gc_bacon")]
        {
            usize::from(rustpython_vm::object::gc::collect()) as i32
        }
        #[cfg(not(feature = "gc_bacon"))]
        {
            0
        }
    }

    #[pyfunction]
    fn isenabled(_args: FuncArgs, _vm: &VirtualMachine) -> bool {
        #[cfg(feature = "gc_bacon")]
        {
            rustpython_vm::object::gc::isenabled()
        }
        #[cfg(not(feature = "gc_bacon"))]
        {
            false
        }
    }

    #[pyfunction]
    fn enable(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        #[cfg(feature = "gc_bacon")]
        {
            rustpython_vm::object::gc::enable();
            Ok(vm.new_pyobj(true))
        }
        #[cfg(not(feature = "gc_bacon"))]
        {
            Err(vm.new_not_implemented_error("".to_owned()))
        }
    }

    #[pyfunction]
    fn disable(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        #[cfg(feature = "gc_bacon")]
        {
            rustpython_vm::object::gc::disable();
            Ok(vm.new_pyobj(true))
        }
        #[cfg(not(feature = "gc_bacon"))]
        {
            Err(vm.new_not_implemented_error("".to_owned()))
        }
    }

    #[pyfunction]
    fn get_count(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
    }

    #[pyfunction]
    fn get_debug(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
    }

    #[pyfunction]
    fn get_objects(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
    }

    #[pyfunction]
    fn get_refererts(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
    }

    #[pyfunction]
    fn get_referrers(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
    }

    #[pyfunction]
    fn get_stats(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
    }

    #[pyfunction]
    fn get_threshold(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
    }

    #[pyfunction]
    fn is_tracked(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
    }

    #[pyfunction]
    fn set_debug(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
    }

    #[pyfunction]
    fn set_threshold(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
    }
}
