use crate::function::PyFuncArgs;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::version;
use crate::vm::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "platform", {
        "python_branch" => ctx.new_rustfunc(platform_python_branch),
        "python_build" => ctx.new_rustfunc(platform_python_build),
        "python_compiler" => ctx.new_rustfunc(platform_python_compiler),
        "python_implementation" => ctx.new_rustfunc(platform_python_implementation),
        "python_revision" => ctx.new_rustfunc(platform_python_revision),
        "python_version" => ctx.new_rustfunc(platform_python_version),
    })
}

fn platform_python_implementation(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    Ok(vm.new_str("RustPython".to_string()))
}

fn platform_python_version(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    Ok(vm.new_str(version::get_version_number()))
}

fn platform_python_compiler(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    Ok(vm.new_str(version::get_compiler()))
}

fn platform_python_build(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    let git_hash = version::get_git_identifier();
    let git_timestamp = version::get_git_datetime();
    Ok(vm
        .ctx
        .new_tuple(vec![vm.new_str(git_hash), vm.new_str(git_timestamp)]))
}

fn platform_python_branch(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    Ok(vm.new_str(version::get_git_branch()))
}

fn platform_python_revision(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    Ok(vm.new_str(version::get_git_revision()))
}
