//! Import mechanics

use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    builtins::{PyCode, PyStr, PyStrRef, PyUtf8Str, traceback::PyTraceback},
    exceptions::types::PyBaseException,
    scope::Scope,
    vm::{VirtualMachine, resolve_frozen_alias, thread},
};

use core::{cell::Cell, sync::atomic::Ordering, time::Duration};
use std::{io::Write, time::Instant};

/// Nested imports are timed per thread, so concurrent imports do not subtract
/// each other's elapsed time. The output header belongs to the interpreter.
#[derive(Clone, Copy, Default)]
pub(crate) struct ImportTimingState {
    depth: usize,
    children: Duration,
}

fn import_time_header(vm: &VirtualMachine) {
    let mut stderr = std::io::stderr().lock();
    if !vm.state.import_time_header.swap(true, Ordering::Relaxed) {
        let _ = writeln!(
            stderr,
            "import time: self [us] | cumulative | imported package"
        );
    }
}

fn import_time_name(name: &Py<PyStr>) -> String {
    use core::fmt::Write;
    let mut output = String::with_capacity(name.as_bytes().len());
    for codepoint in name.as_wtf8().code_points() {
        match codepoint.to_char() {
            Some(ch) => output.push(ch),
            None => {
                let _ = write!(output, "\\u{:04x}", codepoint.to_u32());
            }
        }
    }
    output
}

pub(crate) fn report_cached_import(name: &Py<PyStr>, vm: &VirtualMachine) {
    if vm.state.config.settings.import_time == 2 {
        import_time_header(vm);
        let width = vm.import_timing.get().depth * 2;
        let _ = writeln!(
            std::io::stderr(),
            "import time: cached    | cached     | {:>width$}",
            import_time_name(name)
        );
    }
}

struct ImportTimer<'a> {
    state: &'a Cell<ImportTimingState>,
    parent: ImportTimingState,
    start: Instant,
    name: &'a Py<PyStr>,
}

impl<'a> ImportTimer<'a> {
    fn start(name: &'a Py<PyStr>, vm: &'a VirtualMachine) -> Option<Self> {
        if vm.state.config.settings.import_time == 0 {
            return None;
        }
        import_time_header(vm);
        let parent = vm.import_timing.replace(ImportTimingState {
            depth: vm.import_timing.get().depth + 1,
            children: Duration::ZERO,
        });
        Some(Self {
            state: &vm.import_timing,
            parent,
            start: Instant::now(),
            name,
        })
    }
}

impl Drop for ImportTimer<'_> {
    fn drop(&mut self) {
        let cumulative = self.start.elapsed();
        let own = cumulative.saturating_sub(self.state.get().children);
        self.state.set(ImportTimingState {
            depth: self.parent.depth,
            children: self.parent.children.saturating_add(cumulative),
        });
        let width = self.parent.depth * 2;
        let _ = writeln!(
            std::io::stderr(),
            "import time: {:9} | {:10} | {:width$}{}",
            own.as_nanos().div_ceil(1000),
            cumulative.as_nanos().div_ceil(1000),
            "",
            import_time_name(self.name),
        );
    }
}

pub(crate) fn find_and_load(
    name: &Py<PyStr>,
    function: &'static str,
    vm: &VirtualMachine,
) -> PyResult {
    let _timer = ImportTimer::start(name, vm);
    vm.importlib
        .get_attr(function, vm)?
        .call((name.to_owned(), vm.import_func.clone()), vm)
}

pub(crate) fn check_pyc_magic_number_bytes(buf: &[u8]) -> bool {
    buf.starts_with(&crate::version::PYC_MAGIC_NUMBER_BYTES)
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
    // Importlib callbacks must use the native import function, whose cache-hit
    // path accepts partially initialized modules in concurrent circular imports.
    vm.import_func = vm.builtins.get_attr(identifier!(vm, __import__), vm)?;
    vm.importlib = importlib.clone();
    Ok(importlib)
}

#[cfg(feature = "host_env")]
pub(crate) fn init_importlib_package(vm: &VirtualMachine, importlib: &PyObject) -> PyResult<()> {
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
            vm.ctx.new_utf8_str(name),
        )
    })?;
    Ok(PyCode::new_ref_from_frozen(vm, frozen.code))
}

pub fn import_frozen(vm: &VirtualMachine, module_name: &str) -> PyResult {
    let frozen = vm.state.frozen.get(module_name).ok_or_else(|| {
        vm.new_import_error(
            format!("No such frozen object named {module_name}"),
            vm.ctx.new_utf8_str(module_name),
        )
    })?;
    let module = import_code_obj(
        vm,
        module_name,
        PyCode::new_ref_from_frozen(vm, frozen.code),
        false,
    )?;
    debug_assert!(module.get_attr(identifier!(vm, __name__), vm).is_ok());
    let origname = resolve_frozen_alias(module_name);
    module.set_attr("__origname__", vm.ctx.new_utf8_str(origname), vm)?;
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
        vm.ctx.new_utf8_str(module_name),
    ))
}

#[cfg(feature = "rustpython-compiler")]
pub fn import_file(
    vm: &VirtualMachine,
    module_name: &str,
    file_path: &str,
    content: &str,
) -> PyResult {
    let code = vm
        .compile_with_opts(
            content,
            crate::compiler::Mode::Exec,
            file_path,
            vm.compile_opts(),
        )
        .map_err(|err| err.into_pyexception(vm, Some(content)))?;
    import_code_obj(vm, module_name, code, true)
}

#[cfg(feature = "rustpython-compiler")]
pub fn import_source(vm: &VirtualMachine, module_name: &str, content: &str) -> PyResult {
    let code = vm
        .compile_with_opts(
            content,
            crate::compiler::Mode::Exec,
            "<source>",
            vm.compile_opts(),
        )
        .map_err(|err| err.into_pyexception(vm, Some(content)))?;
    import_code_obj(vm, module_name, code, false)
}

/// Check whether `module.__spec__._initializing` is true, i.e. the module
/// is currently in the middle of being executed for the first time and is
/// not yet safe to hand out as a finished result (used both by the slow
/// import path below and by [`crate::VirtualMachine::import`]'s
/// `sys.modules`-cache fast path).
pub(crate) fn is_module_initializing(module: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
    match vm.get_attribute_opt(module, vm.ctx.intern_str("__spec__"))? {
        Some(spec) => match vm.get_attribute_opt(&spec, vm.ctx.intern_str("_initializing"))? {
            Some(v) => v.try_to_bool(vm),
            None => Ok(false),
        },
        None => Ok(false),
    }
}

/// If `__spec__._initializing` is true, wait for the module to finish
/// initializing by calling `_lock_unlock_module`.
fn import_ensure_initialized(
    module: &PyObject,
    name: &Py<PyStr>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if is_module_initializing(module, vm)? {
        let lock_unlock = vm.importlib.get_attr("_lock_unlock_module", vm)?;
        lock_unlock.call((name.to_owned(),), vm)?;
    }
    report_cached_import(name, vm);
    Ok(())
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
        vm.ctx.new_utf8_str(module_name).into(),
        vm,
    )?;
    if set_file_attr {
        attrs.set_item(
            identifier!(vm, __file__),
            code_obj.source_path().to_object(),
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

    let file_name = traceback.frame.iframe().code().source_path().as_str();

    let (inner_tb, mut now_in_importlib) =
        remove_importlib_frames_inner(vm, traceback.next.lock().clone(), always_trim);
    if file_name == "<frozen importlib._bootstrap>"
        || file_name == "<frozen importlib._bootstrap_external>"
        || file_name == "_frozen_importlib"
        || file_name == "_frozen_importlib_external"
    {
        if traceback.frame.iframe().code().obj_name.as_str() == "_call_with_frames_removed" {
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

    if let Some(tb) = exc.traceback() {
        let trimmed_tb = remove_importlib_frames_inner(vm, Some(tb), always_trim).0;
        exc.set_traceback(trimmed_tb);
    }
}

/// Get origin path from a module spec, checking has_location first.
pub(crate) fn get_spec_file_origin(spec: Option<&PyObject>, vm: &VirtualMachine) -> Option<String> {
    let spec = spec?;

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
        parent.parent().unwrap_or_else(|| Path::new(""))
    } else {
        parent
    };

    // Compute original sys.path[0] from sys.argv[0] (the script path).
    // See: config->sys_path_0, which is set once
    // at initialization and never changes even if sys.path is modified.
    let sys_path_0 = (|| -> Option<String> {
        let argv = vm.sys_module.get_attr("argv", vm).ok()?;
        let argv0 = argv.get_item(&0usize, vm).ok()?;
        let argv0_str = argv0.downcast_ref::<PyUtf8Str>()?;
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
        match crate::host_env::os::current_dir() {
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
pub(crate) fn is_stdlib_module_name(name: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
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
    let result = vm.call_method(&stdlib_names, "__contains__", (name.to_owned(),))?;
    result.try_to_bool(vm)
}

/// Resolve an import target without finding or executing its module.
pub(crate) fn absolute_import_name(
    name: &Py<crate::builtins::PyStr>,
    globals: Option<&PyObject>,
    level: i32,
    vm: &VirtualMachine,
) -> PyResult<crate::builtins::PyStrRef> {
    if level < 0 {
        return Err(vm.new_value_error("level must be >= 0"));
    }
    if level == 0 {
        if name.is_empty() {
            return Err(vm.new_value_error("Empty module name"));
        }
        return Ok(name.to_owned());
    }
    let globals = globals
        .ok_or_else(|| vm.new_key_error(vm.ctx.new_str("'__name__' not in globals").into()))?;
    let empty: PyObjectRef;
    let globals = if vm.is_none(globals) {
        empty = vm.ctx.new_dict().into();
        &empty
    } else {
        globals
    };
    let package = calc_package(Some(globals), vm)?;
    if package.is_empty() {
        return Err(vm.new_import_error(
            "attempted relative import with no known parent package",
            vm.ctx.new_utf8_str(""),
        ));
    }
    resolve_name(name, &package, level as usize, vm)
}

/// PyImport_ImportModuleLevelObject
pub(crate) fn import_module_level(
    name: &Py<PyStr>,
    globals: Option<&PyObject>,
    fromlist: Option<PyObjectRef>,
    level: i32,
    vm: &VirtualMachine,
) -> PyResult {
    if level < 0 {
        return Err(vm.new_value_error("level must be >= 0"));
    }

    // Resolve absolute name
    let abs_name = if level > 0 {
        // When globals is not provided (Rust None), raise KeyError
        // matching resolve_name() where globals==NULL
        let Some(globals_ref) = globals else {
            return Err(vm.new_key_error(vm.ctx.new_str("'__name__' not in globals").into()));
        };
        // When globals is Python None, treat like empty mapping
        let empty_dict_obj: PyObjectRef;
        let globals_ref = if vm.is_none(globals_ref) {
            empty_dict_obj = vm.ctx.new_dict().into();
            &empty_dict_obj
        } else {
            globals_ref
        };
        let package = calc_package(Some(globals_ref), vm)?;
        if package.is_empty() {
            return Err(vm.new_import_error(
                "attempted relative import with no known parent package",
                vm.ctx.new_utf8_str(""),
            ));
        }
        resolve_name(name, &package, level as usize, vm)?
    } else {
        if name.is_empty() {
            return Err(vm.new_value_error("Empty module name"));
        }
        name.to_owned()
    };

    // import_get_module + import_find_and_load
    let sys_modules = vm.sys_module.get_attr("modules", vm)?;
    let module = match sys_modules.get_item(&*abs_name, vm) {
        Ok(m) if !vm.is_none(&m) => {
            import_ensure_initialized(&m, &abs_name, vm)?;
            crate::lazy_import::clear_submodule(&abs_name, true, vm)?;
            m
        }
        Ok(_) => find_and_load(&abs_name, "_find_and_load", vm)?,
        Err(error) if error.fast_isinstance(vm.ctx.exceptions.key_error) => {
            find_and_load(&abs_name, "_find_and_load", vm)?
        }
        Err(error) => return Err(error),
    };

    // Handle fromlist
    let has_from = match fromlist.as_ref().filter(|fl| !vm.is_none(fl)) {
        Some(fl) => fl.try_to_bool(vm)?,
        None => false,
    };

    if has_from {
        let fromlist = fromlist.unwrap();
        // Only call _handle_fromlist if the module looks like a package
        // (has __path__). Non-module objects without __name__/__path__ would
        // crash inside _handle_fromlist; IMPORT_FROM handles per-attribute
        // errors with proper ImportError conversion.
        let has_path = vm
            .get_attribute_opt(&module, vm.ctx.intern_str("__path__"))?
            .is_some();
        if has_path {
            let handle_fromlist = vm.importlib.get_attr("_handle_fromlist", vm)?;
            handle_fromlist.call((module, fromlist, vm.import_func.clone()), vm)
        } else {
            Ok(module)
        }
    } else if level == 0 || !name.is_empty() {
        let name_bytes = name.as_bytes();
        match name_bytes.iter().position(|&byte| byte == b'.') {
            None => Ok(module),
            Some(dot) => {
                let to_return = if level == 0 {
                    vm.ctx.new_str(&name.as_wtf8()[..dot])
                } else {
                    let cut_off = name_bytes.len() - dot;
                    let abs = abs_name.as_wtf8();
                    vm.ctx.new_str(&abs[..abs.len() - cut_off])
                };
                match sys_modules.get_item(&*to_return, vm) {
                    Ok(m) => Ok(m),
                    Err(_) if level == 0 => {
                        // For absolute imports (level 0), try importing the
                        // parent. Matches _bootstrap.__import__ behavior.
                        find_and_load(&to_return, "_find_and_load", vm)
                    }
                    Err(_) => {
                        // For relative imports (level > 0), raise KeyError
                        let to_return_obj: PyObjectRef = vm
                            .ctx
                            .new_utf8_str(format!("'{to_return}' not in sys.modules as expected"))
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
fn resolve_name(
    name: &Py<PyStr>,
    package: &Py<PyStr>,
    level: usize,
    vm: &VirtualMachine,
) -> PyResult<PyStrRef> {
    // ASCII dots are character boundaries in WTF-8 too. Keep the original
    // Python string, including surrogates, while walking up package components.
    let package_bytes = package.as_bytes();
    let mut end = package_bytes.len();
    for _ in 1..level {
        end = package_bytes[..end]
            .iter()
            .rposition(|&byte| byte == b'.')
            .ok_or_else(|| {
                vm.new_import_error(
                    "attempted relative import beyond top-level package",
                    name.to_owned(),
                )
            })?;
    }
    let base = &package.as_wtf8()[..end];
    let abs_name = if name.is_empty() {
        if end == package_bytes.len() {
            package.to_owned()
        } else {
            vm.ctx.new_str(base)
        }
    } else {
        let mut absolute = base.to_owned();
        absolute.push_str(".");
        absolute.push_wtf8(name.as_wtf8());
        vm.ctx.new_str(absolute)
    };
    Ok(abs_name)
}

/// _calc___package__ - calculate package from globals for relative imports
fn calc_package(globals: Option<&PyObject>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
    let globals = globals.ok_or_else(|| {
        vm.new_import_error(
            "attempted relative import with no known parent package",
            vm.ctx.new_utf8_str(""),
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
            .map_err(|_| vm.new_type_error("package must be a string"))?;
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
                .repr_utf8(vm)
                .map(|s| s.as_str().to_owned())
                .unwrap_or_default();
            let msg = format!(
                "__package__ != __spec__.parent ('{}' != {})",
                pkg_str.as_wtf8(),
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
        return Ok(pkg_str);
    } else if let Some(ref spec) = spec
        && !vm.is_none(spec)
        && let Ok(parent) = spec.get_attr("parent", vm)
        && !vm.is_none(&parent)
    {
        let parent_str: PyStrRef = parent
            .downcast()
            .map_err(|_| vm.new_type_error("package set to non-string"))?;
        return Ok(parent_str);
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
            "attempted relative import with no known parent package",
            vm.ctx.new_utf8_str(""),
        )
    })?;
    let mod_name_str: PyStrRef = mod_name
        .downcast()
        .map_err(|_| vm.new_type_error("__name__ must be a string"))?;
    // If not a package (no __path__), strip last component.
    // Uses rpartition('.')[0] semantics: returns empty string when no dot.
    if globals.get_item("__path__", vm).is_err() {
        let name = mod_name_str.as_wtf8();
        Ok(
            match name.as_bytes().iter().rposition(|&byte| byte == b'.') {
                Some(dot) => vm.ctx.new_str(&name[..dot]),
                None => vm.ctx.new_str(""),
            },
        )
    } else {
        Ok(mod_name_str)
    }
}

pub(crate) fn import_from_attribute(
    module: &PyObject,
    name: &Py<crate::builtins::PyStr>,
    vm: &VirtualMachine,
) -> PyResult {
    // Load attribute, and transform any error into import error.
    if let Some(obj) = vm.get_attribute_opt(module, name)? {
        return Ok(obj);
    }
    // fallback to importing '{module.__name__}.{name}' from sys.modules
    let fallback_module = (|| {
        let mod_name = module.get_attr(identifier!(vm, __name__), vm).ok()?;
        let mod_name = mod_name.downcast_ref::<PyUtf8Str>()?;
        let full_mod_name = vm.ctx.new_utf8_str(format!("{}.{name}", mod_name.as_str()));
        let sys_modules = vm.sys_module.get_attr("modules", vm).ok()?;
        sys_modules.get_item(&*full_mod_name, vm).ok()
    })();

    if let Some(sub_module) = fallback_module {
        return Ok(sub_module);
    }

    use crate::import::{get_spec_file_origin, is_possibly_shadowing_path, is_stdlib_module_name};

    // Get module name for the error message
    let mod_name_obj = module.get_attr(identifier!(vm, __name__), vm).ok();
    let mod_name = mod_name_obj
        .as_ref()
        .and_then(|n| n.downcast_ref::<PyUtf8Str>());
    let module_name = mod_name.map_or("<unknown module name>", |s| s.as_str());

    let spec = module
        .get_attr("__spec__", vm)
        .ok()
        .filter(|s| !vm.is_none(s));

    let origin = get_spec_file_origin(spec.as_deref(), vm);

    let is_possibly_shadowing = origin
        .as_ref()
        .is_some_and(|o| is_possibly_shadowing_path(o, vm));
    let is_possibly_shadowing_stdlib = if is_possibly_shadowing {
        if let Some(ref mod_name) = mod_name_obj {
            is_stdlib_module_name(mod_name, vm)?
        } else {
            false
        }
    } else {
        false
    };

    let msg = if is_possibly_shadowing_stdlib {
        let origin = origin.as_ref().unwrap();
        format!(
            "cannot import name '{name}' from '{module_name}' \
                 (consider renaming '{origin}' since it has the same \
                 name as the standard library module named '{module_name}' \
                 and prevents importing that standard library module)"
        )
    } else {
        let is_init = is_module_initializing(module, vm).unwrap_or(false);
        if is_init {
            if is_possibly_shadowing {
                let origin = origin.as_ref().unwrap();
                format!(
                    "cannot import name '{name}' from '{module_name}' \
                         (consider renaming '{origin}' if it has the same name \
                         as a library you intended to import)"
                )
            } else if let Some(ref path) = origin {
                format!(
                    "cannot import name '{name}' from partially initialized module \
                         '{module_name}' (most likely due to a circular import) ({path})"
                )
            } else {
                format!(
                    "cannot import name '{name}' from partially initialized module \
                         '{module_name}' (most likely due to a circular import)"
                )
            }
        } else if let Some(ref path) = origin {
            format!("cannot import name '{name}' from '{module_name}' ({path})")
        } else {
            format!("cannot import name '{name}' from '{module_name}' (unknown location)")
        }
    };
    let err = vm.new_import_error(
        msg,
        match mod_name {
            Some(s) => s.to_owned().into_wtf8(),
            None => vm.ctx.new_utf8_str("<unknown module name>").into_wtf8(),
        },
    );

    if let Some(ref path) = origin {
        let _ignore = err
            .as_object()
            .set_attr("path", vm.ctx.new_str(path.as_str()), vm);
    }

    // name_from = the attribute name that failed to import (best-effort metadata)
    let _ignore = err.as_object().set_attr("name_from", name.to_owned(), vm);

    Err(err)
}
