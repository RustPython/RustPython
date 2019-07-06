use crate::function::PyFuncArgs;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "platform", {
        "python_compiler" => ctx.new_rustfunc(platform_python_compiler),
        "python_implementation" => ctx.new_rustfunc(platform_python_implementation),
        "python_version" => ctx.new_rustfunc(platform_python_version),
    })
}

fn platform_python_implementation(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    Ok(vm.new_str("RustPython".to_string()))
}

fn platform_python_version(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    // TODO: fetch version from somewhere.
    Ok(vm.new_str("4.0.0".to_string()))
}

fn platform_python_compiler(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    let version = rustc_version_runtime::version_meta();
    Ok(vm.new_str(format!("rustc {}", version.semver)))
}
