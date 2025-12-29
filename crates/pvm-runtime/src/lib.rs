mod host;
mod module;

use pvm_host::{Bytes, HostApi, HostError};
use rustpython::InterpreterConfig;
use rustpython_vm::{
    PyResult, Settings, VirtualMachine,
    builtins::PyNone,
    compiler::Mode,
    scope::Scope,
};

pub fn execute_tx(host: &mut dyn HostApi, code: &[u8], input: &[u8]) -> Result<Bytes, HostError> {
    let source = std::str::from_utf8(code).map_err(|_| HostError::InvalidInput)?;
    let mut settings = Settings::default();
    settings.argv = vec!["<pvm>".to_owned()];

    let _host_guard = host::HostGuard::install(host);

    let mut config = InterpreterConfig::new().settings(settings);
    #[cfg(feature = "stdlib")]
    {
        config = config.init_stdlib();
    }
    config = config.add_native_module("pvm_host".to_owned(), module::make_module);
    let interpreter = config.interpreter();

    let result = interpreter.enter(|vm| {
        let res = run_source(vm, source, input);
        if let Err(err) = &res {
            vm.print_exception(err.clone());
        }
        res
    });

    match result {
        Ok(bytes) => Ok(bytes),
        Err(_) => Err(HostError::Internal),
    }
}

fn run_source(vm: &VirtualMachine, source: &str, input: &[u8]) -> PyResult<Bytes> {
    let scope = setup_main_module(vm)?;
    scope
        .globals
        .set_item("__pvm_input__", vm.ctx.new_bytes(input.to_vec()).into(), vm)?;

    let code_obj = vm
        .compile(source, Mode::Exec, "<pvm>".to_owned())
        .map_err(|err| vm.new_syntax_error(&err, Some(source)))?;
    vm.run_code_obj(code_obj, scope.clone())?;

    extract_output(vm, &scope)
}

fn setup_main_module(vm: &VirtualMachine) -> PyResult<Scope> {
    let scope = vm.new_scope_with_builtins();
    let main_module = vm.new_module("__main__", scope.globals.clone(), None);
    main_module
        .dict()
        .set_item("__annotations__", vm.ctx.new_dict().into(), vm)
        .expect("Failed to initialize __main__.__annotations__");

    vm.sys_module
        .get_attr("modules", vm)?
        .set_item("__main__", main_module.into(), vm)?;

    Ok(scope)
}

fn extract_output(vm: &VirtualMachine, scope: &Scope) -> PyResult<Bytes> {
    let output = scope
        .globals
        .get_item_opt("__pvm_output__", vm)?;

    let Some(output) = output else {
        return Ok(Vec::new());
    };

    if output.downcast_ref::<PyNone>().is_some() {
        return Ok(Vec::new());
    }

    output.try_bytes_like(vm, |bytes| bytes.to_vec())
}
