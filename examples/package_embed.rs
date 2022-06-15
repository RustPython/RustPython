use rustpython_vm as vm;
use vm::{builtins::PyStrRef, Interpreter};

fn py_main(interp: &Interpreter) -> vm::PyResult<PyStrRef> {
    interp.enter(|vm| {
        vm.insert_sys_path(vm.new_pyobj("examples"))
            .expect("add path");
        let module = vm.import("package_embed", None, 0)?;
        let name_func = module.get_attr("context", vm)?;
        let result = vm.invoke(&name_func, ())?;
        let result: PyStrRef = result.get_attr("name", vm)?.try_into_value(vm)?;
        vm::PyResult::Ok(result)
    })
}

fn main() -> vm::PyResult<()> {
    let interp = vm::Interpreter::with_init(Default::default(), |vm| {
        vm.add_native_modules(rustpython_stdlib::get_module_inits());
    });
    let result = py_main(&interp);
    let result = result.and_then(|result| {
        println!("name: {}", result);
        Ok(())
    });
    let exit_code = interp.run(|_vm| result);
    std::process::exit(exit_code);
}
