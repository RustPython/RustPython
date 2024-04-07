use rustpython_vm as vm;
use std::process::ExitCode;
use vm::{builtins::PyStrRef, Interpreter};

fn py_main(interp: &Interpreter) -> vm::PyResult<PyStrRef> {
    interp.enter(|vm| {
        // Add local library path
        vm.insert_sys_path(vm.new_pyobj("examples"))
            .expect("add examples to sys.path failed");
        let module = vm.import("package_embed", 0)?;
        let name_func = module.get_attr("context", vm)?;
        let result = name_func.call((), vm)?;
        let result: PyStrRef = result.get_attr("name", vm)?.try_into_value(vm)?;
        vm::PyResult::Ok(result)
    })
}

fn main() -> ExitCode {
    // Add standard library path
    let mut settings = vm::Settings::default();
    settings.path_list.push("Lib".to_owned());
    let interp = vm::Interpreter::with_init(settings, |vm| {
        vm.add_native_modules(rustpython_stdlib::get_module_inits());
    });
    let result = py_main(&interp);
    let result = result.map(|result| {
        println!("name: {result}");
    });
    ExitCode::from(interp.run(|_vm| result))
}
