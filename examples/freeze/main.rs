use rustpython_vm as vm;

fn main() -> vm::PyResult<()> {
    vm::Interpreter::without_stdlib(Default::default()).enter(run)
}

fn run(vm: &vm::VirtualMachine) -> vm::PyResult<()> {
    let scope = vm.new_scope_with_builtins();

    // the file parameter is relative to the directory where the crate's Cargo.toml is located, see $CARGO_MANIFEST_DIR:
    // https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-crates
    let module = vm::py_compile!(file = "examples/freeze/freeze.py");

    let res = vm.run_code_obj(vm.ctx.new_code(module), scope);

    if let Err(exc) = res {
        vm.print_exception(exc);
    }

    Ok(())
}
