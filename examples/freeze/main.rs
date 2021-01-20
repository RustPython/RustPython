use rustpython_vm as vm;

fn main() -> vm::pyobject::PyResult<()> {
    vm::Interpreter::default().enter(run)
}

fn run(vm: &vm::VirtualMachine) -> vm::pyobject::PyResult<()> {
    let scope = vm.new_scope_with_builtins();

    // the file parameter is relevant to the directory where the crate's Cargo.toml is located, see $CARGO_MANIFEST_DIR:
    // https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-crates
    let module = vm::py_compile!(file = "examples/freeze/freeze.py");

    let res = vm.run_code_obj(vm.new_code_object(module), scope);

    if let Err(err) = res {
        vm::exceptions::print_exception(&vm, err);
    }

    Ok(())
}
