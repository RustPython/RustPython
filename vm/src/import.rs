/*
 * Import mechanics
 */
#[cfg(feature = "rustpython-compiler")]
use crate::compile;
use crate::{
    builtins::{code, code::CodeObject, list, traceback::PyTraceback, PyBaseExceptionRef},
    scope::Scope,
    version::get_git_revision,
    vm::{thread, VirtualMachine},
    AsObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
};
use rand::Rng;

pub(crate) fn init_importlib(
    vm: &mut VirtualMachine,
    allow_external_library: bool,
) -> PyResult<()> {
    let importlib = init_importlib_base(vm)?;
    if allow_external_library && cfg!(feature = "rustpython-compiler") {
        init_importlib_package(vm, importlib)?;
    }
    Ok(())
}

pub(crate) fn init_importlib_base(vm: &mut VirtualMachine) -> PyResult<PyObjectRef> {
    flame_guard!("init importlib");

    // importlib_bootstrap needs these and it inlines checks to sys.modules before calling into
    // import machinery, so this should bring some speedup
    #[cfg(all(feature = "threading", not(target_os = "wasi")))]
    import_builtin(vm, "_thread")?;
    import_builtin(vm, "_warnings")?;
    import_builtin(vm, "_weakref")?;

    let importlib = thread::enter_vm(vm, || {
        let importlib = import_frozen(vm, "_frozen_importlib")?;
        let impmod = import_builtin(vm, "_imp")?;
        let install = importlib.get_attr("_install", vm)?;
        vm.invoke(&install, (vm.sys_module.clone(), impmod))?;
        Ok(importlib)
    })?;
    vm.import_func = importlib.get_attr("__import__", vm)?;
    Ok(importlib)
}

pub(crate) fn init_importlib_package(
    vm: &mut VirtualMachine,
    importlib: PyObjectRef,
) -> PyResult<()> {
    thread::enter_vm(vm, || {
        flame_guard!("install_external");

        // same deal as imports above
        #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
        import_builtin(vm, crate::stdlib::os::MODULE_NAME)?;
        #[cfg(windows)]
        import_builtin(vm, "winreg")?;
        import_builtin(vm, "_io")?;
        import_builtin(vm, "marshal")?;

        let install_external = importlib.get_attr("_install_external_importers", vm)?;
        vm.invoke(&install_external, ())?;
        // Set pyc magic number to commit hash. Should be changed when bytecode will be more stable.
        let importlib_external = vm.import("_frozen_importlib_external", None, 0)?;
        let mut magic = get_git_revision().into_bytes();
        magic.truncate(4);
        if magic.len() != 4 {
            magic = rand::thread_rng().gen::<[u8; 4]>().to_vec();
        }
        let magic: PyObjectRef = vm.ctx.new_bytes(magic).into();
        importlib_external.set_attr("MAGIC_NUMBER", magic, vm)?;
        let zipimport_res = (|| -> PyResult<()> {
            let zipimport = vm.import("zipimport", None, 0)?;
            let zipimporter = zipimport.get_attr("zipimporter", vm)?;
            let path_hooks = vm.sys_module.get_attr("path_hooks", vm)?;
            let path_hooks = list::PyListRef::try_from_object(vm, path_hooks)?;
            path_hooks.insert(0, zipimporter);
            Ok(())
        })();
        if zipimport_res.is_err() {
            warn!("couldn't init zipimport")
        }
        Ok(())
    })
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
        .module_inits
        .get(module_name)
        .ok_or_else(|| {
            vm.new_import_error(
                format!("Cannot import builtin module {}", module_name),
                module_name,
            )
        })
        .and_then(|make_module_func| {
            let module = make_module_func(vm);
            let sys_modules = vm.sys_module.get_attr("modules", vm)?;
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
    attrs.set_item("__name__", vm.ctx.new_str(module_name).into(), vm)?;
    if set_file_attr {
        attrs.set_item("__file__", code_obj.source_path.clone().into(), vm)?;
    }
    let module = vm.new_module(module_name, attrs.clone(), None);

    // Store module in cache to prevent infinite loop with mutual importing libs:
    let sys_modules = vm.sys_module.get_attr("modules", vm)?;
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
    tb: Option<PyRef<PyTraceback>>,
    always_trim: bool,
) -> (Option<PyRef<PyTraceback>>, bool) {
    let traceback = if let Some(tb) = tb {
        tb
    } else {
        return (None, false);
    };

    let file_name = traceback.frame.code.source_path.as_str();

    let (inner_tb, mut now_in_importlib) =
        remove_importlib_frames_inner(vm, traceback.next.clone(), always_trim);
    if file_name == "_frozen_importlib" || file_name == "_frozen_importlib_external" {
        if traceback.frame.code.obj_name.as_str() == "_call_with_frames_removed" {
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
    let always_trim = exc.fast_isinstance(&vm.ctx.exceptions.import_error);

    if let Some(tb) = exc.traceback() {
        let trimmed_tb = remove_importlib_frames_inner(vm, Some(tb), always_trim).0;
        exc.set_traceback(trimmed_tb);
    }
    exc.clone()
}
