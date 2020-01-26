///! This example show cases a very simple REPL.
///! While a much better REPL can be found in ../src/shell,
///! This much smaller REPL is still a useful example because it showcases inserting
///! values and functions into the Python runtime's scope, and showcases use
///! of the compilation mode "Single".
///! Note that in particular this REPL does a horrible job of showing users their errors
///! (instead it simply crashes).
use rustpython_compiler::compile;
use rustpython_vm::{pyobject::PyResult, PySettings, VirtualMachine};
// this needs to be in scope in order to insert things into scope.globals
use rustpython_vm::pyobject::ItemProtocol;

fn main() -> PyResult<()> {
    let mut on = true;

    let mut input = String::with_capacity(50);
    let stdin = std::io::stdin();

    let vm = VirtualMachine::new(PySettings::default());
    let scope = vm.new_scope_with_builtins();

    // typing `quit()` is too long, let's make `on(False)` work instead.
    scope
        .globals
        .set_item(
            "on",
            vm.context().new_rustfunc({
                let on: *mut bool = &mut on;
                move |b: bool, _: &VirtualMachine| unsafe { *on = b }
            }),
            &vm,
        )
        .unwrap();

    while on {
        input.clear();
        stdin.read_line(&mut input).unwrap();

        let code_obj = vm
            .compile(&input, compile::Mode::Single, "<embedded>".to_string())
            .map_err(|err| vm.new_syntax_error(&err))?;

        // this line also automatically prints the output
        // (note that this is only the case when compile::Mode::Single is passed to vm.compile)
        let output = vm.run_code_obj(code_obj, scope.clone())?;

        // store the last value in the "last" variable
        if !vm.is_none(&output) {
            scope.globals.set_item("last", output, &vm).unwrap();
        }
    }

    Ok(())
}
