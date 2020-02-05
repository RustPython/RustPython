use rustpython_compiler as compiler;
use rustpython_vm as vm;

fn main() -> vm::pyobject::PyResult<()> {
    let vm = vm::VirtualMachine::new(vm::PySettings::default());

    let scope = vm.new_scope_with_builtins();

    let code_obj = vm
        .compile(
            r#"print("Hello World!")"#,
            compiler::compile::Mode::Exec,
            "<embedded>".to_owned(),
        )
        .map_err(|err| vm.new_syntax_error(&err))?;

    vm.run_code_obj(code_obj, scope)?;

    Ok(())
}
