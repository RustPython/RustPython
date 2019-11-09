/*
 * Import mechanics
 */
use rand::Rng;

use crate::bytecode::CodeObject;
use crate::obj::objtraceback::{PyTraceback, PyTracebackRef};
use crate::obj::{objcode, objtype};
use crate::pyobject::{ItemProtocol, PyObjectRef, PyResult, PyValue};
use crate::scope::Scope;
use crate::version::get_git_revision;
use crate::vm::VirtualMachine;
#[cfg(feature = "rustpython-compiler")]
use rustpython_compiler::compile;

pub fn init_importlib(vm: &VirtualMachine, external: bool) -> PyResult {
    flame_guard!("init importlib");
    let importlib = import_frozen(vm, "_frozen_importlib")?;
    let impmod = import_builtin(vm, "_imp")?;
    let install = vm.get_attribute(importlib.clone(), "_install")?;
    vm.invoke(&install, vec![vm.sys_module.clone(), impmod])?;
    vm.import_func
        .replace(vm.get_attribute(importlib.clone(), "__import__")?);
    if external && cfg!(feature = "rustpython-compiler") {
        flame_guard!("install_external");
        let install_external =
            vm.get_attribute(importlib.clone(), "_install_external_importers")?;
        vm.invoke(&install_external, vec![])?;
        // Set pyc magic number to commit hash. Should be changed when bytecode will be more stable.
        let importlib_external = vm.import("_frozen_importlib_external", &[], 0)?;
        let mut magic = get_git_revision().into_bytes();
        magic.truncate(4);
        if magic.len() != 4 {
            magic = rand::thread_rng().gen::<[u8; 4]>().to_vec();
        }
        vm.set_attr(&importlib_external, "MAGIC_NUMBER", vm.ctx.new_bytes(magic))?;
    }
    Ok(vm.get_none())
}

pub fn import_frozen(vm: &VirtualMachine, module_name: &str) -> PyResult {
    vm.frozen
        .borrow()
        .get(module_name)
        .ok_or_else(|| vm.new_import_error(format!("Cannot import frozen module {}", module_name)))
        .and_then(|frozen| import_codeobj(vm, module_name, frozen.code.clone(), false))
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
    let code_obj = compile::compile(
        &content,
        compile::Mode::Exec,
        file_path,
        vm.settings.optimize,
    )
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
    let module = vm.new_module(module_name, attrs.clone());

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

fn remove_importlib_frames_inner(
    vm: &VirtualMachine,
    tb: Option<PyTracebackRef>,
    always_trim: bool,
) -> Option<PyTracebackRef> {
    let traceback = tb.as_ref()?;
    let file_name = traceback.frame.code.source_path.to_string();
    if (file_name == "_frozen_importlib" || file_name == "_frozen_importlib_external")
        && (always_trim || traceback.frame.code.obj_name == "_call_with_frames_removed")
    {
        return remove_importlib_frames_inner(vm, traceback.next.as_ref().cloned(), always_trim);
    }

    Some(
        PyTraceback::new(
            remove_importlib_frames_inner(vm, traceback.next.as_ref().cloned(), always_trim),
            traceback.frame.clone(),
            traceback.lasti,
            traceback.lineno,
        )
        .into_ref(vm),
    )
}

// TODO: This function should do nothing on verbose mode.
// TODO: Fix this function after making PyTraceback.next mutable
pub fn remove_importlib_frames(vm: &VirtualMachine, exc: &PyObjectRef) -> PyObjectRef {
    let always_trim = objtype::isinstance(exc, &vm.ctx.exceptions.import_error);

    if let Ok(tb) = vm.get_attribute(exc.clone(), "__traceback__") {
        let base_tb: PyTracebackRef = tb.downcast().expect("must be a traceback object");
        let trimed_tb = remove_importlib_frames_inner(vm, Some(base_tb), always_trim)
            .map_or(vm.get_none(), |x| x.into_object());
        vm.set_attr(exc, "__traceback__", trimed_tb).unwrap();
    }
    exc.clone()
}
