/*
 * Import mechanics
 */
use rand::Rng;

use crate::builtins::code::CodeObject;
use crate::builtins::traceback::{PyTraceback, PyTracebackRef};
use crate::builtins::{code, list};
#[cfg(feature = "rustpython-compiler")]
use crate::compile;
use crate::exceptions::PyBaseExceptionRef;
use crate::pyobject::{BorrowValue, ItemProtocol, PyResult, PyValue, TryFromObject, TypeProtocol};
use crate::scope::Scope;
use crate::version::get_git_revision;
use crate::vm::{InitParameter, VirtualMachine};

pub(crate) fn init_importlib(
    vm: &mut VirtualMachine,
    initialize_parameter: InitParameter,
) -> PyResult<()> {
    use crate::vm::thread::enter_vm;
    flame_guard!("init importlib");

    let importlib = enter_vm(vm, || {
        let importlib = import_frozen(vm, "_frozen_importlib")?;
        let impmod = import_builtin(vm, "_imp")?;
        let install = vm.get_attribute(importlib.clone(), "_install")?;
        vm.invoke(&install, vec![vm.sys_module.clone(), impmod])?;
        Ok(importlib)
    })?;
    vm.import_func = vm.get_attribute(importlib.clone(), "__import__")?;

    if initialize_parameter == InitParameter::External && cfg!(feature = "rustpython-compiler") {
        enter_vm(vm, || {
            flame_guard!("install_external");
            let install_external = vm.get_attribute(importlib, "_install_external_importers")?;
            vm.invoke(&install_external, ())?;
            // Set pyc magic number to commit hash. Should be changed when bytecode will be more stable.
            let importlib_external = vm.import("_frozen_importlib_external", None, 0)?;
            let mut magic = get_git_revision().into_bytes();
            magic.truncate(4);
            if magic.len() != 4 {
                magic = rand::thread_rng().gen::<[u8; 4]>().to_vec();
            }
            vm.set_attr(&importlib_external, "MAGIC_NUMBER", vm.ctx.new_bytes(magic))?;
            let zipimport_res = (|| -> PyResult<()> {
                let zipimport = vm.import("zipimport", None, 0)?;
                let zipimporter = vm.get_attribute(zipimport, "zipimporter")?;
                let path_hooks = vm.get_attribute(vm.sys_module.clone(), "path_hooks")?;
                let path_hooks = list::PyListRef::try_from_object(vm, path_hooks)?;
                path_hooks.insert(0, zipimporter);
                Ok(())
            })();
            if zipimport_res.is_err() {
                warn!("couldn't init zipimport")
            }
            Ok(())
        })?
    }
    Ok(())
}

pub fn import_frozen(vm: &VirtualMachine, module_name: &str) -> PyResult {
    vm.state
        .frozen
        .get(module_name)
        .ok_or_else(|| {
            vm.new_import_error(
                format!("Cannot import frozen module {}", module_name),
                module_name,
            )
        })
        .and_then(|frozen| import_codeobj(vm, module_name, frozen.code.clone(), false))
}

pub fn import_builtin(vm: &VirtualMachine, module_name: &str) -> PyResult {
    vm.state
        .stdlib_inits
        .get(module_name)
        .ok_or_else(|| {
            vm.new_import_error(
                format!("Cannot import bultin module {}", module_name),
                module_name,
            )
        })
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
    let code_obj = compile::compile(&content, compile::Mode::Exec, file_path, vm.compile_opts())
        .map_err(|err| vm.new_syntax_error(&err))?;
    import_codeobj(vm, module_name, vm.map_codeobj(code_obj), true)
}

pub fn import_codeobj(
    vm: &VirtualMachine,
    module_name: &str,
    code_obj: CodeObject,
    set_file_attr: bool,
) -> PyResult {
    let attrs = vm.ctx.new_dict();
    attrs.set_item("__name__", vm.ctx.new_str(module_name), vm)?;
    if set_file_attr {
        attrs.set_item("__file__", vm.ctx.new_str(&code_obj.source_path), vm)?;
    }
    let module = vm.new_module(module_name, attrs.clone());

    // Store module in cache to prevent infinite loop with mutual importing libs:
    let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules")?;
    sys_modules.set_item(module_name, module.clone(), vm)?;

    // Execute main code in module:
    vm.run_code_obj(
        code::PyCode::new(code_obj).into_ref(vm),
        Scope::with_builtins(None, attrs, vm),
    )?;
    Ok(module)
}

fn remove_importlib_frames_inner(
    vm: &VirtualMachine,
    tb: Option<PyTracebackRef>,
    always_trim: bool,
) -> (Option<PyTracebackRef>, bool) {
    let traceback = if let Some(tb) = tb {
        tb
    } else {
        return (None, false);
    };

    let file_name = traceback.frame.code.source_path.borrow_value();

    let (inner_tb, mut now_in_importlib) =
        remove_importlib_frames_inner(vm, traceback.next.clone(), always_trim);
    if file_name == "_frozen_importlib" || file_name == "_frozen_importlib_external" {
        if traceback.frame.code.obj_name.borrow_value() == "_call_with_frames_removed" {
            now_in_importlib = true;
        }
        if always_trim || now_in_importlib {
            return (inner_tb, now_in_importlib);
        }
    } else {
        now_in_importlib = false;
    }

    (
        Some(
            PyTraceback::new(
                inner_tb,
                traceback.frame.clone(),
                traceback.lasti,
                traceback.lineno,
            )
            .into_ref(vm),
        ),
        now_in_importlib,
    )
}

// TODO: This function should do nothing on verbose mode.
// TODO: Fix this function after making PyTraceback.next mutable
pub fn remove_importlib_frames(
    vm: &VirtualMachine,
    exc: &PyBaseExceptionRef,
) -> PyBaseExceptionRef {
    let always_trim = exc.isinstance(&vm.ctx.exceptions.import_error);

    if let Some(tb) = exc.traceback() {
        let trimmed_tb = remove_importlib_frames_inner(vm, Some(tb), always_trim).0;
        exc.set_traceback(trimmed_tb);
    }
    exc.clone()
}
