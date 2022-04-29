use rustpython_vm as vm;
use vm::builtins::PyStrRef;

fn main() -> vm::PyResult<()> {
    let interp = vm::Interpreter::with_init(Default::default(), |vm| {
        vm.add_native_modules(rustpython_stdlib::get_module_inits());
    });
    let result = interp
        .run_or_else(
            |vm, exc| {
                vm.print_exception(exc.clone());
                Err(exc)
            },
            |vm| {
                vm.insert_sys_path(vm.new_pyobj("examples"))
                    .expect("add path");
                let module = vm.import("kybra_like", None, 0)?;
                let name_func = module.get_attr("name", vm)?;
                let result = vm.invoke(&name_func, ())?;
                let result: PyStrRef = result.try_into_value(vm)?;
                Ok(result)
            },
        )
        .unwrap();
    println!("name: {}", &result);
    Ok(())
}
