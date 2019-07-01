/*
 * Import mechanics
 */

use std::path::PathBuf;

use crate::bytecode::CodeObject;
use crate::frame::Scope;
use crate::obj::{objcode, objsequence, objstr};
use crate::pyobject::{ItemProtocol, PyResult, PyValue};
use crate::util;
use crate::vm::VirtualMachine;
#[cfg(feature = "rustpython-compiler")]
use rustpython_compiler::compile;

pub fn init_importlib(vm: &VirtualMachine) -> PyResult {
    let importlib = import_frozen(vm, "_frozen_importlib")?;
    let impmod = import_builtin(vm, "_imp")?;
    let install = vm.get_attribute(importlib.clone(), "_install")?;
    vm.invoke(install, vec![vm.sys_module.clone(), impmod])?;
    vm.import_func
        .replace(vm.get_attribute(importlib.clone(), "__import__")?);
    let install_external = vm.get_attribute(importlib.clone(), "_install_external_importers")?;
    vm.invoke(install_external, vec![])?;
    Ok(vm.get_none())
}

pub fn import_frozen(vm: &VirtualMachine, module_name: &str) -> PyResult {
    vm.frozen
        .borrow()
        .get(module_name)
        .ok_or_else(|| vm.new_import_error(format!("Cannot import frozen module {}", module_name)))
        .and_then(|frozen| import_codeobj(vm, module_name, frozen.clone(), false))
}

pub fn import_builtin(vm: &VirtualMachine, module_name: &str) -> PyResult {
    vm.stdlib_inits
        .borrow()
        .get(module_name)
        .ok_or_else(|| vm.new_import_error(format!("Cannot import bultin module {}", module_name)))
        .and_then(|make_module_func| {
            let module = make_module_func(vm);
            let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules")?;
            sys_modules.set_item(module_name, module.clone(), vm)?;
            Ok(module)
        })
}

pub fn import_module(vm: &VirtualMachine, current_path: PathBuf, module_name: &str) -> PyResult {
    // Cached modules:
    let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules").unwrap();

    // First, see if we already loaded the module:
    if let Ok(module) = sys_modules.get_item(module_name.to_string(), vm) {
        Ok(module)
    } else if vm.frozen.borrow().contains_key(module_name) {
        import_frozen(vm, module_name)
    } else if vm.stdlib_inits.borrow().contains_key(module_name) {
        import_builtin(vm, module_name)
    } else if cfg!(feature = "rustpython-compiler") {
        let notfound_error = &vm.ctx.exceptions.module_not_found_error;
        let import_error = &vm.ctx.exceptions.import_error;

        // Time to search for module in any place:
        let file_path = find_source(vm, current_path, module_name)
            .map_err(|e| vm.new_exception(notfound_error.clone(), e))?;
        let source = util::read_file(file_path.as_path())
            .map_err(|e| vm.new_exception(import_error.clone(), e.to_string()))?;

        import_file(
            vm,
            module_name,
            file_path.to_str().unwrap().to_string(),
            source,
        )
    } else {
        let notfound_error = &vm.ctx.exceptions.module_not_found_error;
        Err(vm.new_exception(notfound_error.clone(), module_name.to_string()))
    }
}

#[cfg(feature = "rustpython-compiler")]
pub fn import_file(
    vm: &VirtualMachine,
    module_name: &str,
    file_path: String,
    content: String,
) -> PyResult {
    let code_obj = compile::compile(&content, &compile::Mode::Exec, file_path)
        .map_err(|err| vm.new_syntax_error(&err))?;
    import_codeobj(vm, module_name, code_obj, true)
}

pub fn import_codeobj(
    vm: &VirtualMachine,
    module_name: &str,
    code_obj: CodeObject,
    set_file_attr: bool,
) -> PyResult {
    let attrs = vm.ctx.new_dict();
    attrs.set_item("__name__", vm.new_str(module_name.to_string()), vm)?;
    if set_file_attr {
        attrs.set_item("__file__", vm.new_str(code_obj.source_path.to_owned()), vm)?;
    }
    let module = vm.ctx.new_module(module_name, attrs.clone());

    // Store module in cache to prevent infinite loop with mutual importing libs:
    let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules")?;
    sys_modules.set_item(module_name, module.clone(), vm)?;

    // Execute main code in module:
    vm.run_code_obj(
        objcode::PyCode::new(code_obj).into_ref(vm),
        Scope::with_builtins(None, attrs, vm),
    )?;
    Ok(module)
}

fn find_source(vm: &VirtualMachine, current_path: PathBuf, name: &str) -> Result<PathBuf, String> {
    let sys_path = vm.get_attribute(vm.sys_module.clone(), "path").unwrap();
    let mut paths: Vec<PathBuf> = objsequence::get_elements_list(&sys_path)
        .iter()
        .map(|item| PathBuf::from(objstr::get_value(item)))
        .collect();

    paths.insert(0, current_path);

    let rel_name = name.replace('.', "/");
    let suffixes = [".py", "/__init__.py"];
    let mut file_paths = vec![];
    for path in paths {
        for suffix in suffixes.iter() {
            let mut file_path = path.clone();
            file_path.push(format!("{}{}", rel_name, suffix));
            file_paths.push(file_path);
        }
    }

    match file_paths.iter().find(|p| p.exists()) {
        Some(path) => Ok(path.to_path_buf()),
        None => Err(format!("No module named '{}'", name)),
    }
}
