pub(crate) use gc::make_module;

#[pymodule]
mod gc {
    use crate::vm::{function::FuncArgs, PyResult, VirtualMachine};

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
        Err(vm.new_not_implemented_error("".to_owned()))
    }

    #[pyfunction]
    fn disable(_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error("".to_owned()))
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
