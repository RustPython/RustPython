/*
 * Import mechanics
 */

use std::error::Error;
use std::path::PathBuf;

use super::compile;
use super::pyobject::{AttributeProtocol, DictProtocol, PyResult};
use super::util;
use super::vm::VirtualMachine;
use obj::{objsequence, objstr};

fn import_uncached_module(
    vm: &mut VirtualMachine,
    current_path: PathBuf,
    module: &str,
) -> PyResult {
    // Check for Rust-native modules
    if let Some(module) = vm.stdlib_inits.get(module) {
        return Ok(module(&vm.ctx).clone());
    }

    let notfound_error = vm.context().exceptions.module_not_found_error.clone();
    let import_error = vm.context().exceptions.import_error.clone();

    // Time to search for module in any place:
    let file_path = find_source(vm, current_path, module)
        .map_err(|e| vm.new_exception(notfound_error.clone(), e))?;
    let source = util::read_file(file_path.as_path())
        .map_err(|e| vm.new_exception(import_error.clone(), e.description().to_string()))?;
    let code_obj = compile::compile(
        &source,
        &compile::Mode::Exec,
        file_path.to_str().unwrap().to_string(),
        vm.ctx.code_type(),
    )
    .map_err(|err| {
        let syntax_error = vm.context().exceptions.syntax_error.clone();
        vm.new_exception(syntax_error, err.description().to_string())
    })?;
    // trace!("Code object: {:?}", code_obj);

    let builtins = vm.get_builtin_scope();
    let scope = vm.ctx.new_scope(Some(builtins));
    vm.ctx
        .set_attr(&scope, "__name__", vm.new_str(module.to_string()));
    vm.run_code_obj(code_obj, scope.clone())?;
    Ok(vm.ctx.new_module(module, scope))
}

pub fn import_module(
    vm: &mut VirtualMachine,
    current_path: PathBuf,
    module_name: &str,
) -> PyResult {
    // First, see if we already loaded the module:
    let sys_modules = vm.sys_module.get_attr("modules").unwrap();
    if let Some(module) = sys_modules.get_item(module_name) {
        return Ok(module);
    }
    let module = import_uncached_module(vm, current_path, module_name)?;
    vm.ctx.set_item(&sys_modules, module_name, module.clone());
    Ok(module)
}

pub fn import(
    vm: &mut VirtualMachine,
    current_path: PathBuf,
    module_name: &str,
    symbol: &Option<String>,
) -> PyResult {
    let module = import_module(vm, current_path, module_name)?;
    // If we're importing a symbol, look it up and use it, otherwise construct a module and return
    // that
    if let Some(symbol) = symbol {
        module.get_attr(symbol).map_or_else(
            || {
                let import_error = vm.context().exceptions.import_error.clone();
                Err(vm.new_exception(import_error, format!("cannot import name '{}'", symbol)))
            },
            Ok,
        )
    } else {
        Ok(module)
    }
}

fn find_source(vm: &VirtualMachine, current_path: PathBuf, name: &str) -> Result<PathBuf, String> {
    let sys_path = vm.sys_module.get_attr("path").unwrap();
    let mut paths: Vec<PathBuf> = objsequence::get_elements(&sys_path)
        .iter()
        .map(|item| PathBuf::from(objstr::get_value(item)))
        .collect();

    paths.insert(0, current_path);

    let suffixes = [".py", "/__init__.py"];
    let mut file_paths = vec![];
    for path in paths {
        for suffix in suffixes.iter() {
            let mut file_path = path.clone();
            file_path.push(format!("{}{}", name, suffix));
            file_paths.push(file_path);
        }
    }

    match file_paths.iter().filter(|p| p.exists()).next() {
        Some(path) => Ok(path.to_path_buf()),
        None => Err(format!("No module named '{}'", name)),
    }
}
