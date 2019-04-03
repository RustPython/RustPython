/*
 * Import mechanics
 */

use std::error::Error;
use std::path::PathBuf;

use crate::compile;
use crate::frame::Scope;
use crate::obj::{objsequence, objstr};
use crate::pyobject::{DictProtocol, ItemProtocol, PyResult};
use crate::util;
use crate::vm::VirtualMachine;

fn import_uncached_module(vm: &VirtualMachine, current_path: PathBuf, module: &str) -> PyResult {
    // Check for Rust-native modules
    if let Some(module) = vm.stdlib_inits.borrow().get(module) {
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
        vm,
        &source,
        &compile::Mode::Exec,
        file_path.to_str().unwrap().to_string(),
    )
    .map_err(|err| {
        let syntax_error = vm.context().exceptions.syntax_error.clone();
        vm.new_exception(syntax_error, err.description().to_string())
    })?;
    // trace!("Code object: {:?}", code_obj);

    let attrs = vm.ctx.new_dict();
    attrs.set_item(&vm.ctx, "__name__", vm.new_str(module.to_string()));
    vm.run_code_obj(code_obj, Scope::new(None, attrs.clone()))?;
    Ok(vm.ctx.new_module(module, attrs))
}

pub fn import_module(vm: &VirtualMachine, current_path: PathBuf, module_name: &str) -> PyResult {
    // First, see if we already loaded the module:
    let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules")?;
    if let Ok(module) = sys_modules.get_item(module_name.to_string(), vm) {
        return Ok(module);
    }
    let module = import_uncached_module(vm, current_path, module_name)?;
    sys_modules.set_item(module_name, module.clone(), vm)?;
    Ok(module)
}

fn find_source(vm: &VirtualMachine, current_path: PathBuf, name: &str) -> Result<PathBuf, String> {
    let sys_path = vm.get_attribute(vm.sys_module.clone(), "path").unwrap();
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

    match file_paths.iter().find(|p| p.exists()) {
        Some(path) => Ok(path.to_path_buf()),
        None => Err(format!("No module named '{}'", name)),
    }
}
