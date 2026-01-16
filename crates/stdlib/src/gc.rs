pub(crate) use gc::module_def;

#[pymodule]
mod gc {
    use crate::vm::{PyObjectRef, PyResult, VirtualMachine, function::FuncArgs};

    #[pyfunction]
    fn collect(_args: FuncArgs, _vm: &VirtualMachine) -> i32 {
        0
    }

    #[pyfunction]
    fn isenabled(_args: FuncArgs, _vm: &VirtualMachine) -> bool {
        false
    }

    #[pyfunction]
    fn enable(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(""))
    }

    #[pyfunction]
    fn disable(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(""))
    }

    #[pyfunction]
    fn get_count(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(""))
    }

    #[pyfunction]
    fn get_debug(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(""))
    }

    #[pyfunction]
    fn get_objects(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(""))
    }

    #[pyfunction]
    fn get_referents(_args: FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
        // RustPython does not track object references.
        vm.ctx.new_tuple(vec![]).into()
    }

    #[pyfunction]
    fn get_referrers(_args: FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
        // RustPython does not track object references.
        vm.ctx.new_list(vec![]).into()
    }

    #[pyfunction]
    fn get_stats(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(""))
    }

    #[pyfunction]
    fn get_threshold(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(""))
    }

    #[pyfunction]
    fn is_tracked(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(""))
    }

    #[pyfunction]
    fn set_debug(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(""))
    }

    #[pyfunction]
    fn set_threshold(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(""))
    }
}
