use rustpython::InterpreterBuilderExt;
use rustpython_vm as vm;
use std::process::ExitCode;
use vm::Interpreter;

fn py_main(interp: &Interpreter) -> vm::embedding::Result<String> {
    interp.enter(|vm| {
        // Add local library path.
        vm.exec("import sys; sys.path.insert(0, 'examples')")?;
        let module = vm.import("package_embed")?;
        let result = module.get_attr("context")?.call(&[])?;
        result.get_attr("name")?.to_string()
    })
}

fn main() -> ExitCode {
    // Add standard library path.
    let mut settings = vm::Settings::default();
    settings.path_list.push("Lib".to_owned());
    let interp = vm::Interpreter::builder(settings).init_stdlib().build();
    let result = py_main(&interp).map(|name| println!("name: {name}"));
    vm::host_env::os::exit_code(interp.run(|_vm| result).expect("no native workers remain"))
}
