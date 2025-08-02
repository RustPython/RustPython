use rustpython::vm::*;
use rustpython_vm as vm;
use rustpython_vm::builtins::PyModule;

// A global fn called from Python which extracts the id from the module and prints it.
fn print_id(vm: &vm::VirtualMachine) {
    let module = vm.import("__id_module", 0).ok().unwrap();
    let obj = module.get_attr("__id", vm).ok().unwrap();
    let id = obj.try_to_value::<i32>(vm).ok().unwrap();

    println!("The id is {}", id);
}

fn main() -> vm::PyResult<()> {
    vm::Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let scope = vm.new_scope_with_builtins();

        // Register the global function
        let _ =
            scope
                .globals
                .set_item("print_id", vm.new_function("print_id", print_id).into(), vm);

        // Create a module and set an id.
        let module = PyModule::new().into_ref(&vm.ctx);
        module
            .as_object()
            .set_attr("__id", vm.new_pyobj(42_i32), vm)
            .ok()
            .unwrap();

        // Register the module
        let sys = vm.import("sys", 0).ok().unwrap();
        let modules = sys.get_attr("modules", vm).ok().unwrap();
        modules
            .set_item("__id_module", module.into(), vm)
            .ok()
            .unwrap();

        // Execute the code
        let source = r#"print_id()"#;
        let code_obj = vm
            .compile(source, vm::compiler::Mode::Exec, "<embedded>".to_owned())
            .map_err(|err| vm.new_syntax_error(&err, Some(source)))?;

        vm.run_code_obj(code_obj, scope)?;

        Ok(())
    })
}
