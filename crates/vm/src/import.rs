//! Import mechanics

use crate::{
    AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult,
    builtins::{PyCode, PyStr, PyStrRef, traceback::PyTraceback},
    exceptions::types::PyBaseException,
    scope::Scope,
    vm::{VirtualMachine, resolve_frozen_alias, thread},
};

pub(crate) fn check_pyc_magic_number_bytes(buf: &[u8]) -> bool {
    buf.starts_with(&crate::version::PYC_MAGIC_NUMBER_BYTES[..2])
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
        let bootstrap = import_frozen(vm, "_frozen_importlib")?;
        let install = bootstrap.get_attr("_install", vm)?;
        let imp = import_builtin(vm, "_imp")?;
        install.call((vm.sys_module.clone(), imp), vm)?;
        Ok(bootstrap)
    })?;
    vm.import_func = importlib.get_attr(identifier!(vm, __import__), vm)?;
    vm.importlib = importlib.clone();
    Ok(importlib)
}

#[cfg(feature = "host_env")]
pub(crate) fn init_importlib_package(vm: &VirtualMachine, importlib: PyObjectRef) -> PyResult<()> {
    use crate::{TryFromObject, builtins::PyListRef};

    thread::enter_vm(vm, || {
        flame_guard!("install_external");

        // same deal as imports above
        import_builtin(vm, crate::stdlib::os::MODULE_NAME)?;
        #[cfg(windows)]
        import_builtin(vm, "winreg")?;
        import_builtin(vm, "_io")?;
        import_builtin(vm, "marshal")?;

        let install_external = importlib.get_attr("_install_external_importers", vm)?;
        install_external.call((), vm)?;
        let zipimport_res = (|| -> PyResult<()> {
            let zipimport = vm.import("zipimport", 0)?;
            let zipimporter = zipimport.get_attr("zipimporter", vm)?;
            let path_hooks = vm.sys_module.get_attr("path_hooks", vm)?;
            let path_hooks = PyListRef::try_from_object(vm, path_hooks)?;
            path_hooks.insert(0, zipimporter);
            Ok(())
        })();
        if zipimport_res.is_err() {
            warn!("couldn't init zipimport")
        }
        Ok(())
    })
}

pub fn make_frozen(vm: &VirtualMachine, name: &str) -> PyResult<PyRef<PyCode>> {
    let frozen = vm.state.frozen.get(name).ok_or_else(|| {
        vm.new_import_error(
            format!("No such frozen object named {name}"),
            vm.ctx.new_str(name),
        )
    })?;
    Ok(vm.ctx.new_code(frozen.code))
}

pub fn import_frozen(vm: &VirtualMachine, module_name: &str) -> PyResult {
    let frozen = vm.state.frozen.get(module_name).ok_or_else(|| {
        vm.new_import_error(
            format!("No such frozen object named {module_name}"),
            vm.ctx.new_str(module_name),
        )
    })?;
    let module = import_code_obj(vm, module_name, vm.ctx.new_code(frozen.code), false)?;
    debug_assert!(module.get_attr(identifier!(vm, __name__), vm).is_ok());
    let origname = resolve_frozen_alias(module_name);
    module.set_attr("__origname__", vm.ctx.new_str(origname), vm)?;
    Ok(module)
}

pub fn import_builtin(vm: &VirtualMachine, module_name: &str) -> PyResult {
    let sys_modules = vm.sys_module.get_attr("modules", vm)?;

    // Check if already in sys.modules (handles recursive imports)
    if let Ok(module) = sys_modules.get_item(module_name, vm) {
        return Ok(module);
    }

    // Try multi-phase init first (preferred for modules that import other modules)
    if let Some(&def) = vm.state.module_defs.get(module_name) {
        // Phase 1: Create and initialize module
        let module = def.create_module(vm)?;

        // Add to sys.modules BEFORE exec (critical for circular import handling)
        sys_modules.set_item(module_name, module.clone().into(), vm)?;

        // Phase 2: Call exec slot (can safely import other modules now)
        // If exec fails, remove the partially-initialized module from sys.modules
        if let Err(e) = def.exec_module(vm, &module) {
            let _ = sys_modules.del_item(module_name, vm);
            return Err(e);
        }

        return Ok(module.into());
    }

    // Module not found in module_defs
    Err(vm.new_import_error(
        format!("Cannot import builtin module {module_name}"),
        vm.ctx.new_str(module_name),
    ))
}

#[cfg(feature = "rustpython-compiler")]
pub fn import_file(
    vm: &VirtualMachine,
    module_name: &str,
    file_path: String,
    content: &str,
) -> PyResult {
    let code = vm
        .compile_with_opts(
            content,
            crate::compiler::Mode::Exec,
            file_path,
            vm.compile_opts(),
        )
        .map_err(|err| vm.new_syntax_error(&err, Some(content)))?;
    import_code_obj(vm, module_name, code, true)
}

#[cfg(feature = "rustpython-compiler")]
pub fn import_source(vm: &VirtualMachine, module_name: &str, content: &str) -> PyResult {
    let code = vm
        .compile_with_opts(
            content,
            crate::compiler::Mode::Exec,
            "<source>".to_owned(),
            vm.compile_opts(),
        )
        .map_err(|err| vm.new_syntax_error(&err, Some(content)))?;
    import_code_obj(vm, module_name, code, false)
}

pub fn import_code_obj(
    vm: &VirtualMachine,
    module_name: &str,
    code_obj: PyRef<PyCode>,
    set_file_attr: bool,
) -> PyResult {
    let attrs = vm.ctx.new_dict();
    attrs.set_item(
        identifier!(vm, __name__),
        vm.ctx.new_str(module_name).into(),
        vm,
    )?;
    if set_file_attr {
        attrs.set_item(
            identifier!(vm, __file__),
            code_obj.source_path.to_object(),
            vm,
        )?;
    }
    let module = vm.new_module(module_name, attrs.clone(), None);

    // Store module in cache to prevent infinite loop with mutual importing libs:
    let sys_modules = vm.sys_module.get_attr("modules", vm)?;
    sys_modules.set_item(module_name, module.clone().into(), vm)?;

    // Execute main code in module:
    let scope = Scope::with_builtins(None, attrs, vm);
    vm.run_code_obj(code_obj, scope)?;
    Ok(module.into())
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
        remove_importlib_frames_inner(vm, traceback.next.lock().clone(), always_trim);
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
            .into_ref(&vm.ctx),
        ),
        now_in_importlib,
    )
}

// TODO: This function should do nothing on verbose mode.
// TODO: Fix this function after making PyTraceback.next mutable
pub fn remove_importlib_frames(vm: &VirtualMachine, exc: &Py<PyBaseException>) {
    if vm.state.config.settings.verbose != 0 {
        return;
    }

    let always_trim = exc.fast_isinstance(vm.ctx.exceptions.import_error);

    if let Some(tb) = exc.__traceback__() {
        let trimmed_tb = remove_importlib_frames_inner(vm, Some(tb), always_trim).0;
        exc.set_traceback_typed(trimmed_tb);
    }
}

/// Get origin path from a module spec, checking has_location first.
pub(crate) fn get_spec_file_origin(
    spec: &Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> Option<String> {
    let spec = spec.as_ref()?;
    let has_location = spec
        .get_attr("has_location", vm)
        .ok()
        .and_then(|v| v.try_to_bool(vm).ok())
        .unwrap_or(false);
    if !has_location {
        return None;
    }
    spec.get_attr("origin", vm).ok().and_then(|origin| {
        if vm.is_none(&origin) {
            None
        } else {
            origin
                .downcast_ref::<PyStr>()
                .and_then(|s| s.to_str().map(|s| s.to_owned()))
        }
    })
}

/// Check if a module file possibly shadows another module of the same name.
/// Compares the module's directory with the original sys.path[0] (derived from sys.argv[0]).
pub(crate) fn is_possibly_shadowing_path(origin: &str, vm: &VirtualMachine) -> bool {
    use std::path::Path;

    if vm.state.config.settings.safe_path {
        return false;
    }

    let origin_path = Path::new(origin);
    let parent = match origin_path.parent() {
        Some(p) => p,
        None => return false,
    };
    // For packages (__init__.py), look one directory further up
    let root = if origin_path.file_name() == Some("__init__.py".as_ref()) {
        parent.parent().unwrap_or(Path::new(""))
    } else {
        parent
    };

    // Compute original sys.path[0] from sys.argv[0] (the script path).
    // See: config->sys_path_0, which is set once
    // at initialization and never changes even if sys.path is modified.
    let sys_path_0 = (|| -> Option<String> {
        let argv = vm.sys_module.get_attr("argv", vm).ok()?;
        let argv0 = argv.get_item(&0usize, vm).ok()?;
        let argv0_str = argv0.downcast_ref::<PyStr>()?;
        let s = argv0_str.as_str();

        // For -c and REPL, original sys.path[0] is ""
        if s == "-c" || s.is_empty() {
            return Some(String::new());
        }
        // For scripts, original sys.path[0] is dirname(argv[0])
        Some(
            Path::new(s)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_owned(),
        )
    })();

    let sys_path_0 = match sys_path_0 {
        Some(p) => p,
        None => return false,
    };

    let cmp_path = if sys_path_0.is_empty() {
        match std::env::current_dir() {
            Ok(d) => d.to_string_lossy().to_string(),
            Err(_) => return false,
        }
    } else {
        sys_path_0
    };

    root.to_str() == Some(cmp_path.as_str())
}

/// Check if a module name is in sys.stdlib_module_names.
/// Takes the original __name__ object to preserve str subclass behavior.
/// Propagates errors (e.g. TypeError for unhashable str subclass).
pub(crate) fn is_stdlib_module_name(name: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
    let stdlib_names = match vm.sys_module.get_attr("stdlib_module_names", vm) {
        Ok(names) => names,
        Err(_) => return Ok(false),
    };
    if !stdlib_names.class().fast_issubclass(vm.ctx.types.set_type)
        && !stdlib_names
            .class()
            .fast_issubclass(vm.ctx.types.frozenset_type)
    {
        return Ok(false);
    }
    let result = vm.call_method(&stdlib_names, "__contains__", (name.clone(),))?;
    result.try_to_bool(vm)
}

/// PyImport_ImportModuleLevelObject
pub(crate) fn import_module_level(
    name: &Py<PyStr>,
    globals: Option<PyObjectRef>,
    fromlist: Option<PyObjectRef>,
    level: i32,
    vm: &VirtualMachine,
) -> PyResult {
    if level < 0 {
        return Err(vm.new_value_error("level must be >= 0".to_owned()));
    }

    let name_str = match name.to_str() {
        Some(s) => s,
        None => {
            // Name contains surrogates. Like CPython, try sys.modules
            // lookup with the Python string key directly.
            if level == 0 {
                let sys_modules = vm.sys_module.get_attr("modules", vm)?;
                return sys_modules.get_item(name, vm).map_err(|_| {
                    vm.new_import_error(format!("No module named '{}'", name), name.to_owned())
                });
            }
            return Err(vm.new_import_error(format!("No module named '{}'", name), name.to_owned()));
        }
    };

    // Resolve absolute name
    let abs_name = if level > 0 {
        // When globals is not provided (Rust None), raise KeyError
        // matching resolve_name() where globals==NULL
        if globals.is_none() {
            return Err(vm.new_key_error(vm.ctx.new_str("'__name__' not in globals").into()));
        }
        let globals_ref = globals.as_ref().unwrap();
        // When globals is Python None, treat like empty mapping
        let empty_dict_obj;
        let globals_ref = if vm.is_none(globals_ref) {
            empty_dict_obj = vm.ctx.new_dict().into();
            &empty_dict_obj
        } else {
            globals_ref
        };
        let package = calc_package(Some(globals_ref), vm)?;
        if package.is_empty() {
            return Err(vm.new_import_error(
                "attempted relative import with no known parent package".to_owned(),
                vm.ctx.new_str(""),
            ));
        }
        resolve_name(name_str, &package, level as usize, vm)?
    } else {
        if name_str.is_empty() {
            return Err(vm.new_value_error("Empty module name".to_owned()));
        }
        name_str.to_owned()
    };

    // import_get_module + import_find_and_load
    let sys_modules = vm.sys_module.get_attr("modules", vm)?;
    let module = match sys_modules.get_item(&*abs_name, vm) {
        Ok(m) if !vm.is_none(&m) => m,
        _ => {
            let find_and_load = vm.importlib.get_attr("_find_and_load", vm)?;
            let abs_name_obj = vm.ctx.new_str(&*abs_name);
            find_and_load.call((abs_name_obj, vm.import_func.clone()), vm)?
        }
    };

    // Handle fromlist
    let has_from = match fromlist.as_ref().filter(|fl| !vm.is_none(fl)) {
        Some(fl) => fl.clone().try_to_bool(vm)?,
        None => false,
    };

    if has_from {
        let fromlist = fromlist.unwrap();
        // Only call _handle_fromlist if the module looks like a package
        // (has __path__). Non-module objects without __name__/__path__ would
        // crash inside _handle_fromlist; IMPORT_FROM handles per-attribute
        // errors with proper ImportError conversion.
        let has_path = vm
            .get_attribute_opt(module.clone(), vm.ctx.intern_str("__path__"))?
            .is_some();
        if has_path {
            let handle_fromlist = vm.importlib.get_attr("_handle_fromlist", vm)?;
            handle_fromlist.call((module, fromlist, vm.import_func.clone()), vm)
        } else {
            Ok(module)
        }
    } else if level == 0 || !name_str.is_empty() {
        match name_str.find('.') {
            None => Ok(module),
            Some(dot) => {
                let to_return = if level == 0 {
                    name_str[..dot].to_owned()
                } else {
                    let cut_off = name_str.len() - dot;
                    abs_name[..abs_name.len() - cut_off].to_owned()
                };
                match sys_modules.get_item(&*to_return, vm) {
                    Ok(m) => Ok(m),
                    Err(_) if level == 0 => {
                        // For absolute imports (level 0), try importing the
                        // parent. Matches _bootstrap.__import__ behavior.
                        let find_and_load = vm.importlib.get_attr("_find_and_load", vm)?;
                        let to_return_obj = vm.ctx.new_str(&*to_return);
                        find_and_load.call((to_return_obj, vm.import_func.clone()), vm)
                    }
                    Err(_) => {
                        // For relative imports (level > 0), raise KeyError
                        let to_return_obj: PyObjectRef = vm
                            .ctx
                            .new_str(format!("'{to_return}' not in sys.modules as expected"))
                            .into();
                        Err(vm.new_key_error(to_return_obj))
                    }
                }
            }
        }
    } else {
        Ok(module)
    }
}

/// resolve_name in import.c - resolve relative import name
fn resolve_name(name: &str, package: &str, level: usize, vm: &VirtualMachine) -> PyResult<String> {
    // Python: bits = package.rsplit('.', level - 1)
    // Rust: rsplitn(level, '.') gives maxsplit=level-1
    let parts: Vec<&str> = package.rsplitn(level, '.').collect();
    if parts.len() < level {
        return Err(vm.new_import_error(
            "attempted relative import beyond top-level package".to_owned(),
            vm.ctx.new_str(name),
        ));
    }
    // rsplitn returns parts right-to-left, so last() is the leftmost (base)
    let base = parts.last().unwrap();
    if name.is_empty() {
        Ok(base.to_string())
    } else {
        Ok(format!("{base}.{name}"))
    }
}

/// _calc___package__ - calculate package from globals for relative imports
fn calc_package(globals: Option<&PyObjectRef>, vm: &VirtualMachine) -> PyResult<String> {
    let globals = globals.ok_or_else(|| {
        vm.new_import_error(
            "attempted relative import with no known parent package".to_owned(),
            vm.ctx.new_str(""),
        )
    })?;

    let package = globals.get_item("__package__", vm).ok();
    let spec = globals.get_item("__spec__", vm).ok();

    if let Some(ref pkg) = package
        && !vm.is_none(pkg)
    {
        let pkg_str: PyStrRef = pkg
            .clone()
            .downcast()
            .map_err(|_| vm.new_type_error("package must be a string".to_owned()))?;
        // Warn if __package__ != __spec__.parent
        if let Some(ref spec) = spec
            && !vm.is_none(spec)
            && let Ok(parent) = spec.get_attr("parent", vm)
            && !pkg_str.is(&parent)
            && pkg_str
                .as_object()
                .rich_compare_bool(&parent, crate::types::PyComparisonOp::Ne, vm)
                .unwrap_or(false)
        {
            let parent_repr = parent
                .repr(vm)
                .map(|s| s.as_str().to_owned())
                .unwrap_or_default();
            let msg = format!(
                "__package__ != __spec__.parent ('{}' != {})",
                pkg_str.as_str(),
                parent_repr
            );
            let warn = vm
                .import("_warnings", 0)
                .and_then(|w| w.get_attr("warn", vm));
            if let Ok(warn_fn) = warn {
                let _ = warn_fn.call(
                    (
                        vm.ctx.new_str(msg),
                        vm.ctx.exceptions.deprecation_warning.to_owned(),
                    ),
                    vm,
                );
            }
        }
        return Ok(pkg_str.as_str().to_owned());
    } else if let Some(ref spec) = spec
        && !vm.is_none(spec)
        && let Ok(parent) = spec.get_attr("parent", vm)
        && !vm.is_none(&parent)
    {
        let parent_str: PyStrRef = parent
            .downcast()
            .map_err(|_| vm.new_type_error("package set to non-string".to_owned()))?;
        return Ok(parent_str.as_str().to_owned());
    }

    // Fall back to __name__ and __path__
    let warn = vm
        .import("_warnings", 0)
        .and_then(|w| w.get_attr("warn", vm));
    if let Ok(warn_fn) = warn {
        let _ = warn_fn.call(
            (
                vm.ctx.new_str("can't resolve package from __spec__ or __package__, falling back on __name__ and __path__"),
                vm.ctx.exceptions.import_warning.to_owned(),
            ),
            vm,
        );
    }

    let mod_name = globals.get_item("__name__", vm).map_err(|_| {
        vm.new_import_error(
            "attempted relative import with no known parent package".to_owned(),
            vm.ctx.new_str(""),
        )
    })?;
    let mod_name_str: PyStrRef = mod_name
        .downcast()
        .map_err(|_| vm.new_type_error("__name__ must be a string".to_owned()))?;
    let mut package = mod_name_str.as_str().to_owned();
    // If not a package (no __path__), strip last component.
    // Uses rpartition('.')[0] semantics: returns empty string when no dot.
    if globals.get_item("__path__", vm).is_err() {
        package = match package.rfind('.') {
            Some(dot) => package[..dot].to_owned(),
            None => String::new(),
        };
    }
    Ok(package)
}
