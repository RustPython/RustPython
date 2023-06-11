use rustpython_vm as vm;
use std::process::ExitCode;
use vm::{builtins::PyStrRef, Interpreter};

fn py_main(interp: &Interpreter) -> vm::PyResult<PyStrRef> {
    interp.enter(|vm| {
        vm.insert_sys_path(vm.new_pyobj("examples"))
            .expect("add path");
        let module = vm.import("package_embed", None, 0)?;
        let name_func = module.get_attr("context", vm)?;
        let result = name_func.call((), vm)?;
        let result: PyStrRef = result.get_attr("name", vm)?.try_into_value(vm)?;
        vm::PyResult::Ok(result)
    })
}

fn main() -> ExitCode {
    let interp = vm::Interpreter::with_init(Default::default(), |vm| {
        vm.add_native_modules(rustpython_stdlib::get_module_inits());
    });
    let result = py_main(&interp);
    let result = result.map(|result| {
        println!("name: {result}");
    });
    ExitCode::from(interp.run(|_vm| result))
}
