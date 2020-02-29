use std::collections::HashMap;

use rustpython_vm as vm;

fn main() -> vm::pyobject::PyResult<()> {
    let vm = vm::VirtualMachine::new(vm::PySettings::default());

    let scope = vm.new_scope_with_builtins();

    let modules: HashMap<&str, vm::bytecode::FrozenModule> = vm::py_compile_bytecode!(
        source = "print(\"Hello world1!\")\n",
        module_name = "__main__"
    );

    vm.run_code_obj(
        vm.ctx
            .new_code_object(modules.get("__main__").unwrap().code.clone()),
        scope,
    )?;

    Ok(())
}
