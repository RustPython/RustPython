use rustpython::vm::*;
use rustpython_vm::builtins::PyModule;

/// A global fn called from Python which extracts the id from the injected module and returns / prints it to the console.
/// This is useful in a multi-threaded environment where you may have several threads sharing global functions. The
/// id would allow a local context for each thread, for example by using a global Arc<Mutex<Hashmap>>.
fn get_id(vm: &vm::VirtualMachine) -> PyResult<i32> {
    let module = vm.import("__id_module", 0)?;
    let obj = module.get_attr("__id", vm)?;
    let id = obj.try_to_value::<i32>(vm)?;

    println!("The id is {}", id);

    Ok(id)
}

fn main() -> PyResult<()> {
    vm::Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let scope = vm.new_scope_with_builtins();

        // Register the global function
        let _ = scope
            .globals
            .set_item("get_id", vm.new_function("get_id", get_id).into(), vm);

        // Create a module and set an id
        let module = PyModule::new().into_ref(&vm.ctx);
        module
            .as_object()
            .set_attr("__id", vm.new_pyobj(42_i32), vm)?;

        // Register the module
        let sys = vm.import("sys", 0)?;
        let modules = sys.get_attr("modules", vm)?;
        modules.set_item("__id_module", module.into(), vm)?;

        // Execute the code
        let source = r#"get_id()"#;
        let code_obj = vm
            .compile(source, compiler::Mode::Exec, "<embedded>".to_owned())
            .map_err(|err| vm.new_syntax_error(&err, Some(source)))?;

        vm.run_code_obj(code_obj, scope)?;

        Ok(())
    })
}
