use rustpython_compiler::compile;
use rustpython_vm::{pyobject::PyResult, PySettings, VirtualMachine};

fn main() -> PyResult<()> {
    let vm = VirtualMachine::new(PySettings::default());

    let scope = vm.new_scope_with_builtins();

    let code_obj = vm
        .compile(
            r#"print("Hello World!")"#,
            compile::Mode::Exec,
            "<embedded>".to_string(),
        )
        .map_err(|err| vm.new_syntax_error(&err))?;

    vm.run_code_obj(code_obj, scope)?;

    Ok(())
}
