use crate::pyobject::PyObjectRef;
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

fn platform_python_implementation(_vm: &VirtualMachine) -> String {
    "RustPython".to_string()
}

fn platform_python_version(_vm: &VirtualMachine) -> String {
    version::get_version_number()
}

fn platform_python_compiler(_vm: &VirtualMachine) -> String {
    version::get_compiler()
}

fn platform_python_build(_vm: &VirtualMachine) -> (String, String) {
    (version::get_git_identifier(), version::get_git_datetime())
}

fn platform_python_branch(_vm: &VirtualMachine) -> String {
    version::get_git_branch()
}

fn platform_python_revision(_vm: &VirtualMachine) -> String {
    version::get_git_revision()
}
