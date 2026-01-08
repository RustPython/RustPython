mod host;
mod determinism;
mod guard;
mod module;

pub use determinism::DeterminismOptions;
use pvm_host::{Bytes, HostApi, HostError};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use rustpython::InterpreterConfig;
use rustpython_vm::{
    AsObject,
    PyObjectRef, PyResult, Settings, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyListRef, PyNone},
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
    pub determinism: Option<DeterminismOptions>,
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
            determinism: None,
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

    pub fn with_determinism(mut self, determinism: DeterminismOptions) -> Self {
        self.determinism = Some(determinism);
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

    let determinism = options.determinism.clone().or_else(|| {
        if options.deterministic {
            Some(DeterminismOptions::deterministic(options.hash_seed))
        } else {
            None
        }
    });

    if let Some(det) = determinism.as_ref().filter(|item| item.enabled) {
        settings.hash_seed = Some(det.hash_seed);
        settings.ignore_environment = true;
        settings.import_site = false;
        settings.user_site_directory = false;
        settings.isolated = true;
        settings.safe_path = true;
        settings.install_signal_handlers = false;
    } else if let Some(seed) = options.hash_seed {
        settings.hash_seed = Some(seed);
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
        if let Some(det) = determinism.as_ref().filter(|item| item.enabled) {
            if let Err(err) = guard::install(vm, det, options.host_module_name.as_str()) {
                vm.print_exception(err);
                return Err(HostError::Internal);
            }
        }
        let res = run_source(vm, source, input, options);
        let trace_result = determinism
            .as_ref()
            .filter(|item| item.enabled)
            .map(|det| export_import_trace(vm, det))
            .unwrap_or(Ok(()));
        match res {
            Ok(bytes) => {
                if let Err(err) = trace_result {
                    return Err(err);
                }
                Ok(bytes)
            }
            Err(err) => {
                if let Err(trace_err) = trace_result {
                    eprintln!("pvm import trace failed: {trace_err}");
                }
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
    if let Some(host_error) = determinism_error_from_exception(vm, err, options) {
        vm.print_exception(err.clone());
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

fn determinism_error_from_exception(
    vm: &VirtualMachine,
    err: &PyBaseExceptionRef,
    options: &ExecutionOptions,
) -> Option<HostError> {
    let module = get_host_module(vm, options)?;
    let det_err_obj = module.get_attr("DeterministicValidationError", vm).ok()?;
    let det_err_type = rustpython_vm::builtins::PyTypeRef::try_from_object(vm, det_err_obj).ok()?;
    if err.fast_isinstance(&det_err_type) {
        return Some(HostError::InvalidInput);
    }

    let nondet_obj = module.get_attr("NonDeterministicError", vm).ok()?;
    let nondet_type = rustpython_vm::builtins::PyTypeRef::try_from_object(vm, nondet_obj).ok()?;
    if err.fast_isinstance(&nondet_type) {
        return Some(HostError::Forbidden);
    }

    let ooo_obj = module.get_attr("OutOfGasError", vm).ok()?;
    let ooo_type = rustpython_vm::builtins::PyTypeRef::try_from_object(vm, ooo_obj).ok()?;
    if err.fast_isinstance(&ooo_type) {
        return Some(HostError::OutOfGas);
    }

    None
}

fn host_error_from_exception(
    vm: &VirtualMachine,
    err: &PyBaseExceptionRef,
    options: &ExecutionOptions,
) -> Option<HostError> {
    let module = get_host_module(vm, options)?;
    let host_error_obj = module.get_attr("HostError", vm).ok()?;
    let host_error_type =
        rustpython_vm::builtins::PyTypeRef::try_from_object(vm, host_error_obj).ok()?;
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

fn get_host_module(vm: &VirtualMachine, options: &ExecutionOptions) -> Option<PyObjectRef> {
    let modules_obj = vm.sys_module.get_attr("modules", vm).ok()?;
    let modules = rustpython_vm::builtins::PyDictRef::try_from_object(vm, modules_obj).ok()?;
    modules
        .get_item_opt(options.host_module_name.as_str(), vm)
        .ok()
        .flatten()
}

fn export_import_trace(vm: &VirtualMachine, det: &DeterminismOptions) -> Result<(), HostError> {
    if !det.trace_imports {
        return Ok(());
    }
    let Some(path) = det.trace_path.as_ref() else {
        return Ok(());
    };

    let trace = read_trace_list(vm, "_pvm_import_trace")?;
    let blocked = read_trace_list(vm, "_pvm_import_blocked")?;
    let unique = dedup_in_order(&trace);
    let blocked_unique = dedup_in_order(&blocked);

    let blacklist = det.stdlib_blacklist.clone();
    let mut whitelist = det.stdlib_whitelist.clone();
    let mut missing = Vec::new();
    let mut blacklisted = Vec::new();

    for name in &unique {
        if denied_by_list(&blacklist, name) {
            blacklisted.push(name.clone());
            continue;
        }
        if allowed_by_list(&whitelist, name) {
            continue;
        }
        missing.push(name.clone());
        whitelist.push(name.clone());
    }

    let payload = format!(
        "{{\"trace\":{},\"unique\":{},\"blocked\":{},\"missing\":{},\"blacklisted\":{},\"whitelist_base\":{},\"whitelist_suggested\":{},\"blacklist\":{}}}\n",
        json_list(&trace),
        json_list(&unique),
        json_list(&blocked_unique),
        json_list(&missing),
        json_list(&blacklisted),
        json_list(&det.stdlib_whitelist),
        json_list(&whitelist),
        json_list(&blacklist),
    );

    write_trace_file(path, &payload)?;
    Ok(())
}

fn read_trace_list(vm: &VirtualMachine, name: &str) -> Result<Vec<String>, HostError> {
    let name_obj = vm.ctx.new_str(name);
    let obj = vm
        .sys_module
        .get_attr(&name_obj, vm)
        .map_err(|_| HostError::Internal)?;
    let list = PyListRef::try_from_object(vm, obj).map_err(|_| HostError::Internal)?;
    let items = list.borrow_vec();
    let mut out = Vec::with_capacity(items.len());
    for item in items.iter() {
        let value = item.str(vm).map_err(|_| HostError::Internal)?;
        out.push(value.to_string());
    }
    Ok(out)
}

fn dedup_in_order(items: &[String]) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        if seen.insert(item.as_str()) {
            out.push(item.clone());
        }
    }
    out
}

fn allowed_by_list(list: &[String], name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if list.iter().any(|item| item == name) {
        return true;
    }
    let mut prefix = String::new();
    for (idx, part) in name.split('.').enumerate() {
        if idx > 0 {
            prefix.push('.');
        }
        prefix.push_str(part);
        if list.iter().any(|item| item == &prefix) {
            return true;
        }
    }
    let name_prefix = format!("{name}.");
    list.iter().any(|item| item.starts_with(&name_prefix))
}

fn denied_by_list(list: &[String], name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut prefix = String::new();
    for (idx, part) in name.split('.').enumerate() {
        if idx > 0 {
            prefix.push('.');
        }
        prefix.push_str(part);
        if list.iter().any(|item| item == &prefix) {
            return true;
        }
    }
    false
}

fn write_trace_file(path: &str, payload: &str) -> Result<(), HostError> {
    let path = Path::new(path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|_| HostError::Internal)?;
        }
    }
    fs::write(path, payload).map_err(|_| HostError::Internal)?;
    Ok(())
}

fn json_list(items: &[String]) -> String {
    let mut out = String::from("[");
    for (idx, item) in items.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(item));
        out.push('"');
    }
    out.push(']');
    out
}

fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}
