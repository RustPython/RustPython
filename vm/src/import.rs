/*
 * Import mechanics
 */

use crate::bytecode::CodeObject;
use crate::frame::Scope;
use crate::obj::objstr::PyStringRef;
use crate::obj::{objcode, objsequence, objstr, objtype};
use crate::pyobject::{ItemProtocol, PyObjectRef, PyResult, PyValue, TryFromObject};
use crate::vm::VirtualMachine;
#[cfg(feature = "rustpython-compiler")]
use rustpython_compiler::compile;

pub fn init_importlib(vm: &VirtualMachine, external: bool) -> PyResult {
    flame_guard!("init importlib");
    let importlib = import_frozen(vm, "_frozen_importlib")?;
    vm.importlib.replace(importlib.clone());
    let impmod = import_builtin(vm, "_imp")?;
    let install = vm.get_attribute(importlib.clone(), "_install")?;
    vm.invoke(install, vec![vm.sys_module.clone(), impmod])?;
    vm.import_func
        .replace(vm.get_attribute(importlib.clone(), "__import__")?);
    if external && cfg!(feature = "rustpython-compiler") {
        flame_guard!("install_external");
        let install_external =
            vm.get_attribute(importlib.clone(), "_install_external_importers")?;
        vm.invoke(install_external, vec![])?;
    }
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

// TODO: This function should do nothing on verbose mode.
fn remove_importlib_frames(vm: &VirtualMachine, exc: &PyObjectRef) -> PyObjectRef {
    let always_trim = objtype::isinstance(exc, &vm.ctx.exceptions.import_error);

    if let Ok(tb) = vm.get_attribute(exc.clone(), "__traceback__") {
        if objtype::isinstance(&tb, &vm.ctx.list_type()) {
            let tb_entries = objsequence::get_elements_list(&tb).to_vec();
            let mut in_importlib = false;
            let new_tb = tb_entries
                .iter()
                .filter(|tb_entry| {
                    let location_attrs = objsequence::get_elements_tuple(&tb_entry);
                    let file_name = objstr::get_value(&location_attrs[0]);
                    if file_name == "_frozen_importlib" || file_name == "_frozen_importlib_external"
                    {
                        let run_obj_name = objstr::get_value(&location_attrs[2]);
                        if run_obj_name == "_call_with_frames_removed" {
                            in_importlib = true;
                        }
                        !always_trim && !in_importlib
                    } else {
                        in_importlib = false;
                        true
                    }
                })
                .cloned()
                .collect();
            vm.set_attr(exc, "__traceback__", vm.ctx.new_list(new_tb))
                .unwrap();
        }
    }
    exc.clone()
}

fn get_module(name: &str, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
    let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules")?;
    sys_modules.get_item_option(name.to_string(), vm)
}

fn resolve_name(
    name: &str,
    globals: Option<PyObjectRef>,
    level: usize,
    vm: &VirtualMachine,
) -> PyResult<String> {
    let package = if let Some(globals) = globals {
        if vm.isinstance(&globals, &vm.ctx.dict_type())? {
            // TODO: Add checks
            match globals.get_item_option("__package__".to_string(), vm)? {
                Some(package) => Ok(PyStringRef::try_from_object(vm, package)?
                    .as_str()
                    .to_string()),
                None => match globals.get_item_option("__spec__".to_string(), vm)? {
                    Some(spec) => Ok(PyStringRef::try_from_object(
                        vm,
                        spec.get_item("parent", vm)?,
                    )?
                    .as_str()
                    .to_string()),
                    None => {
                        // TODO: Add warning
                        let package = PyStringRef::try_from_object(
                            vm,
                            globals.get_item("__name__".to_string(), vm)?,
                        )?
                        .as_str()
                        .to_string();
                        if globals.get_item_option("__path__", vm)?.is_some() {
                            Ok(package.rsplitn(2, '.').last().unwrap().to_string())
                        } else {
                            Ok(package)
                        }
                    }
                },
            }
        } else {
            Err(vm.new_type_error("globals must be a dict".to_string()))
        }
    } else {
        Err(vm.new_key_error(vm.new_str("'__name__' not in globals".to_string())))
    }?;

    let base = package.rsplitn(level, '.').last().unwrap();

    Ok(format!("{}.{}", base, name))
}

// Rusr implementation of importlib __import__ for optimization.
pub fn import(
    name: &str,
    globals: Option<PyObjectRef>,
    _locals: Option<PyObjectRef>,
    from_list: Option<PyObjectRef>,
    level: usize,
    vm: &VirtualMachine,
) -> PyResult {
    let abs_name = if level > 0 {
        resolve_name(name, globals, level, vm)?
    } else {
        if name.is_empty() {
            return Err(vm.new_value_error("Empty module name".to_string()));
        }
        name.to_string()
    };

    // TODO: call _lock_unlock_module
    let module = match get_module(&abs_name, vm)? {
        Some(module) => module,
        None => {
            let find_and_load =
                vm.get_attribute(vm.importlib.borrow().clone(), "_find_and_load")?;
            vm.invoke(
                find_and_load,
                vec![
                    vm.new_str(abs_name.clone()),
                    vm.import_func.borrow().clone(),
                ],
            )
            .map_err(|exc| remove_importlib_frames(vm, &exc))?
        }
    };

    let has_from = if let Some(ref from_list) = from_list {
        if vm.isinstance(from_list, &vm.ctx.tuple_type())? {
            !objsequence::get_elements_tuple(&from_list).is_empty()
        } else {
            return Err(vm.new_type_error("from_list must be a tuple".to_string()));
        }
    } else {
        false
    };

    if has_from {
        // TODO: Check if error
        if vm.get_attribute(module.clone(), "__path__").is_ok() {
            let handle_fromlist =
                vm.get_attribute(vm.importlib.borrow().clone(), "_handle_fromlist")?;
            vm.invoke(
                handle_fromlist,
                vec![module, from_list.unwrap(), vm.import_func.borrow().clone()],
            )
            .map_err(|exc| remove_importlib_frames(vm, &exc))
        } else {
            Ok(module)
        }
    } else if level == 0 || !name.is_empty() {
        if !name.contains('.') {
            Ok(module)
        } else if level == 0 {
            import(name.splitn(1, '.').next().unwrap(), None, None, None, 0, vm)
        } else {
            let cut = abs_name.len() - (name.len() - name.find('.').unwrap());
            let to_return = &abs_name[0..cut];
            get_module(to_return, vm)?.ok_or_else(|| {
                vm.new_key_error(
                    vm.new_str(format!("{} not in sys.modules as expected", to_return)),
                )
            })
        }
    } else {
        Ok(module)
    }
}
