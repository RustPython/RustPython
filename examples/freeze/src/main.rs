use std::collections::HashMap;

use rustpython_vm as vm;

fn main() -> vm::pyobject::PyResult<()> {
    let vm = vm::VirtualMachine::new(vm::PySettings::default());

    let scope = vm.new_scope_with_builtins();

    let modules: HashMap<String, vm::bytecode::FrozenModule> =
        vm::py_compile_bytecode!(file = "freeze.py");

    let res = vm.run_code_obj(
        vm.ctx
            .new_code_object(modules.get("frozen").unwrap().code.clone()),
        scope,
    );

    if let Err(err) = res {
        vm::exceptions::print_exception(&vm, &err)
    }

    Ok(())
}
