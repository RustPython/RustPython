use rustpython_vm as vm;

fn main() -> vm::PyResult<()> {
    vm::Interpreter::without_stdlib(Default::default()).enter(run)
}

fn run(vm: &vm::VirtualMachine) -> vm::PyResult<()> {
    let scope = vm.new_scope_with_builtins();

    // the file parameter is relative to the current file.
    let module = vm::py_compile!(file = "freeze.py");

    let res = vm.run_code_obj(vm.ctx.new_code(module), scope);

    if let Err(exc) = res {
        vm.print_exception(exc);
    }

    Ok(())
}
