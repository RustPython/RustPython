mod host;
mod module;

use pvm_host::{Bytes, HostApi, HostError};
use rustpython::InterpreterConfig;
use rustpython_vm::{
    AsObject,
    PyResult, Settings, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyNone},
    compiler::Mode,
    convert::TryFromObject,
    scope::Scope,
};

#[derive(Clone, Debug)]
pub struct ExecutionOptions {
    pub argv: Vec<String>,
    pub module_name: String,
    pub source_path: String,
    pub input_var: String,
    pub output_var: String,
    pub entrypoint: Option<String>,
    pub host_module_name: String,
    pub init_stdlib: bool,
    pub deterministic: bool,
    pub hash_seed: Option<u32>,
    pub set_main_module: bool,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            argv: Vec::new(),
            module_name: "__main__".to_owned(),
            source_path: "<pvm>".to_owned(),
            input_var: "__pvm_input__".to_owned(),
            output_var: "__pvm_output__".to_owned(),
            entrypoint: None,
            host_module_name: "pvm_host".to_owned(),
            init_stdlib: true,
            deterministic: false,
            hash_seed: None,
            set_main_module: true,
        }
    }
}

impl ExecutionOptions {
    pub fn with_entrypoint(mut self, entrypoint: impl Into<String>) -> Self {
        self.entrypoint = Some(entrypoint.into());
        self
    }

    pub fn with_module_name(mut self, module_name: impl Into<String>) -> Self {
        self.module_name = module_name.into();
        self
    }

    pub fn with_source_path(mut self, source_path: impl Into<String>) -> Self {
        self.source_path = source_path.into();
        self
    }

    pub fn with_argv(mut self, argv: Vec<String>) -> Self {
        self.argv = argv;
        self
    }

    pub fn deterministic(mut self) -> Self {
        self.deterministic = true;
        self
    }
}

pub fn execute_tx(host: &mut dyn HostApi, code: &[u8], input: &[u8]) -> Result<Bytes, HostError> {
    execute_tx_with_options(host, code, input, &ExecutionOptions::default())
}

pub fn execute_tx_with_options(
    host: &mut dyn HostApi,
    code: &[u8],
    input: &[u8],
    options: &ExecutionOptions,
) -> Result<Bytes, HostError> {
    let source = std::str::from_utf8(code).map_err(|_| HostError::InvalidInput)?;
    let mut settings = Settings::default();
    settings.argv = if options.argv.is_empty() {
        vec![options.source_path.clone()]
    } else {
        options.argv.clone()
    };
    if let Some(seed) = options.hash_seed {
        settings.hash_seed = Some(seed);
    }
    if options.deterministic {
        settings.hash_seed = Some(options.hash_seed.unwrap_or(0));
        settings.ignore_environment = true;
        settings.import_site = false;
        settings.user_site_directory = false;
        settings.isolated = true;
        settings.safe_path = true;
        settings.install_signal_handlers = false;
    }

    let _host_guard = host::HostGuard::install(host);

    let mut config = InterpreterConfig::new().settings(settings);
    #[cfg(feature = "stdlib")]
    {
        if options.init_stdlib {
            config = config.init_stdlib();
        }
    }
    config = config.add_native_module(options.host_module_name.clone(), module::make_module);
    let interpreter = config.interpreter();

    interpreter.enter(|vm| {
        let res = run_source(vm, source, input, options);
        match res {
            Ok(bytes) => Ok(bytes),
            Err(err) => {
                let host_error = map_exception(vm, &err, options);
                if host_error == HostError::Internal {
                    vm.print_exception(err.clone());
                }
                Err(host_error)
            }
        }
    })
}

fn run_source(
    vm: &VirtualMachine,
    source: &str,
    input: &[u8],
    options: &ExecutionOptions,
) -> PyResult<Bytes> {
    let scope = setup_main_module(vm, options)?;
    let input_obj = vm.ctx.new_bytes(input.to_vec());
    scope
        .globals
        .set_item(options.input_var.as_str(), input_obj.clone().into(), vm)?;

    let code_obj = vm
        .compile(source, Mode::Exec, options.source_path.clone())
        .map_err(|err| vm.new_syntax_error(&err, Some(source)))?;
    vm.run_code_obj(code_obj, scope.clone())?;

    let output = if let Some(entrypoint) = &options.entrypoint {
        let callable = scope
            .globals
            .get_item_opt(entrypoint.as_str(), vm)?
            .ok_or_else(|| {
                vm.new_name_error(
                    format!(
                        "pvm entrypoint '{}' not found in module '{}'",
                        entrypoint, options.module_name
                    ),
                    vm.ctx.new_str(entrypoint.as_str()),
                )
            })?;
        Some(callable.call((input_obj,), vm)?)
    } else {
        scope.globals.get_item_opt(options.output_var.as_str(), vm)?
    };

    extract_output(vm, output)
}

fn setup_main_module(vm: &VirtualMachine, options: &ExecutionOptions) -> PyResult<Scope> {
    let scope = vm.new_scope_with_builtins();
    let main_module = vm.new_module(options.module_name.as_str(), scope.globals.clone(), None);
    main_module
        .dict()
        .set_item("__annotations__", vm.ctx.new_dict().into(), vm)
        .expect("Failed to initialize __main__.__annotations__");
    main_module
        .dict()
        .set_item("__file__", vm.ctx.new_str(options.source_path.clone()).into(), vm)
        .expect("Failed to initialize __main__.__file__");
    main_module
        .dict()
        .set_item("__cached__", vm.ctx.none(), vm)
        .expect("Failed to initialize __main__.__cached__");

    let modules = vm.sys_module.get_attr("modules", vm)?;
    modules.set_item(options.module_name.as_str(), main_module.clone().into(), vm)?;
    if options.set_main_module && options.module_name != "__main__" {
        modules.set_item("__main__", main_module.into(), vm)?;
    }

    Ok(scope)
}

fn extract_output(vm: &VirtualMachine, output: Option<rustpython_vm::PyObjectRef>) -> PyResult<Bytes> {
    let Some(output) = output else {
        return Ok(Vec::new());
    };

    if output.downcast_ref::<PyNone>().is_some() {
        return Ok(Vec::new());
    }

    output
        .try_bytes_like(vm, |bytes| bytes.to_vec())
        .map_err(|_| {
            vm.new_type_error("pvm output must be bytes-like or None".to_owned())
        })
}

fn map_exception(
    vm: &VirtualMachine,
    err: &PyBaseExceptionRef,
    options: &ExecutionOptions,
) -> HostError {
    if let Some(host_error) = host_error_from_exception(vm, err, options) {
        return host_error;
    }

    let is_syntax = err.fast_isinstance(vm.ctx.exceptions.syntax_error);
    if is_syntax {
        return HostError::InvalidInput;
    }

    let is_type = err.fast_isinstance(vm.ctx.exceptions.type_error);
    if is_type {
        return HostError::InvalidInput;
    }

    HostError::Internal
}

fn host_error_from_exception(
    vm: &VirtualMachine,
    err: &PyBaseExceptionRef,
    options: &ExecutionOptions,
) -> Option<HostError> {
    let modules_obj = vm.sys_module.get_attr("modules", vm).ok()?;
    let modules = rustpython_vm::builtins::PyDictRef::try_from_object(vm, modules_obj).ok()?;
    let module = modules
        .get_item_opt(options.host_module_name.as_str(), vm)
        .ok()
        .flatten()?;
    let host_error_obj = module.get_attr("HostError", vm).ok()?;
    let host_error_type = rustpython_vm::builtins::PyTypeRef::try_from_object(vm, host_error_obj).ok()?;
    if !err.fast_isinstance(&host_error_type) {
        return None;
    }
    let code_obj = err.as_object().get_attr("code", vm).ok()?;
    let code = u32::try_from_object(vm, code_obj).ok()?;
    HostError::from_code(code).or_else(|| {
        let name_obj = err.as_object().get_attr("name", vm).ok()?;
        let name = name_obj.str(vm).ok()?.to_string();
        HostError::from_name(name.as_str())
    })
}
