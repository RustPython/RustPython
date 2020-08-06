use std::collections::HashMap;

use rustpython_vm as vm;

fn main() -> vm::pyobject::PyResult<()> {
    let vm = vm::VirtualMachine::new(vm::PySettings::default());

    let scope = vm.new_scope_with_builtins();

    // the file parameter is relevant to the directory where the crate's Cargo.toml is located, see $CARGO_MANIFEST_DIR:
    // https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-crates
    let modules: HashMap<String, vm::bytecode::FrozenModule> =
        vm::py_compile_bytecode!(file = "examples/freeze/freeze.py");

    let res = vm.run_code_obj(
        vm.ctx
            .new_code_object(modules.get("frozen").unwrap().code.clone()),
        scope,
    );

    if let Err(err) = res {
        vm::exceptions::print_exception(&vm, err);
    }

    Ok(())
}
