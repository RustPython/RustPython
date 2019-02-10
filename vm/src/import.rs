/*
 * Import mechanics
 */

extern crate rustpython_parser;

use std::path::PathBuf;

use self::rustpython_parser::parser;
use super::compile;
use super::pyobject::{DictProtocol, PyResult};
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
    let filepath = find_source(vm, current_path, module)
        .map_err(|e| vm.new_exception(notfound_error.clone(), e))?;
    let source = parser::read_file(filepath.as_path())
        .map_err(|e| vm.new_exception(import_error.clone(), e))?;

    let code_obj = compile::compile(
        vm,
        &source,
        &compile::Mode::Exec,
        filepath.to_str().unwrap().to_string(),
    )?;
    // trace!("Code object: {:?}", code_obj);

    let builtins = vm.get_builtin_scope();
    let scope = vm.ctx.new_scope(Some(builtins));
    vm.ctx
        .set_item(&scope, "__name__", vm.new_str(module.to_string()));
    vm.run_code_obj(code_obj, scope.clone())?;
    Ok(vm.ctx.new_module(module, scope))
}

pub fn import_module(
    vm: &mut VirtualMachine,
    current_path: PathBuf,
    module_name: &str,
) -> PyResult {
    // First, see if we already loaded the module:
    let sys_modules = vm.sys_module.get_item("modules").unwrap();
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
        module.get_item(symbol).map_or_else(
            || {
                let import_error = vm.context().exceptions.import_error.clone();
                Err(vm.new_exception(import_error, format!("cannot import name '{}'", symbol)))
            },
            |obj| Ok(obj),
        )
    } else {
        Ok(module)
    }
}

fn find_source(vm: &VirtualMachine, current_path: PathBuf, name: &str) -> Result<PathBuf, String> {
    let sys_path = vm.sys_module.get_item("path").unwrap();
    let mut paths: Vec<PathBuf> = objsequence::get_elements(&sys_path)
        .iter()
        .map(|item| PathBuf::from(objstr::get_value(item)))
        .collect();

    paths.insert(0, current_path);

    let suffixes = [".py", "/__init__.py"];
    let mut filepaths = vec![];
    for path in paths {
        for suffix in suffixes.iter() {
            let mut filepath = path.clone();
            filepath.push(format!("{}{}", name, suffix));
            filepaths.push(filepath);
        }
    }

    match filepaths.iter().find(|p| p.exists()) {
        Some(path) => Ok(path.to_path_buf()),
        None => Err(format!("No module named '{}'", name)),
    }
}
