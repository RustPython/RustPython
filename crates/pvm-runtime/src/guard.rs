use crate::determinism::DeterminismOptions;
use rustpython_vm::{
    PyObjectRef, PyResult, VirtualMachine,
    builtins::PyListRef,
    compiler::Mode,
};

const GUARD_SOURCE: &str = r#"
import builtins
import sys

_ALLOW = set(PVM_WHITELIST)
_DENY = set(PVM_BLACKLIST)
_REAL_IMPORT = builtins.__import__
_HOST = _REAL_IMPORT(PVM_HOST_MODULE, None, None, (), 0)
_TRACE_IMPORTS = bool(PVM_TRACE_IMPORTS)
_TRACE_ALLOW_ALL = bool(PVM_TRACE_ALLOW_ALL)
_TRACE = []
_TRACE_BLOCKED = []
sys._pvm_import_trace = _TRACE
sys._pvm_import_blocked = _TRACE_BLOCKED

_ALIAS = {
    "time": "pvm_sdk.pvm_time",
    "random": "pvm_sdk.pvm_random",
    "pvm_time": "pvm_sdk.pvm_time",
    "pvm_random": "pvm_sdk.pvm_random",
    "pvm_sys": "pvm_sdk.pvm_sys",
}


def _resolve_name(name, globals, level):
    if level and globals:
        pkg = globals.get("__package__") or globals.get("__name__")
        if pkg:
            parts = pkg.split(".")
            if level <= len(parts):
                base = ".".join(parts[: len(parts) - level + 1])
                return base + ("." + name if name else "")
    return name


def _is_allowed(name, globals=None, level=0):
    resolved = _resolve_name(name, globals, level)
    if resolved == "sys" and globals:
        importer = globals.get("__package__") or globals.get("__name__")
        if importer and _allowed_by_whitelist(importer):
            return True
    parts = resolved.split(".") if resolved else []
    for i in range(1, len(parts) + 1):
        prefix = ".".join(parts[:i])
        if prefix in _DENY:
            return False
    if resolved in _DENY:
        return False
    if resolved in _ALLOW:
        return True
    for i in range(1, len(parts) + 1):
        prefix = ".".join(parts[:i])
        if prefix in _ALLOW:
            return True
    if resolved:
        prefix = resolved + "."
        for item in _ALLOW:
            if item.startswith(prefix):
                return True
    return False


def _allowed_by_whitelist(name):
    parts = name.split(".") if name else []
    if name in _ALLOW:
        return True
    for i in range(1, len(parts) + 1):
        prefix = ".".join(parts[:i])
        if prefix in _ALLOW:
            return True
    if name:
        prefix = name + "."
        for item in _ALLOW:
            if item.startswith(prefix):
                return True
    return False


def _alias(name, target):
    try:
        if "." in target:
            leaf = target.rsplit(".", 1)[-1]
            mod = _REAL_IMPORT(target, None, None, (leaf,), 0)
        else:
            mod = _REAL_IMPORT(target, None, None, (), 0)
    except Exception:
        return
    sys.modules[name] = mod


def _record_import(name, allowed):
    if not _TRACE_IMPORTS:
        return
    if not name:
        return
    _TRACE.append(name)
    if not allowed:
        _TRACE_BLOCKED.append(name)


if PVM_SYS_PATH is not None:
    sys.path[:] = PVM_SYS_PATH
    try:
        sys.path_importer_cache.clear()
    except Exception:
        sys.path_importer_cache = {}

for _name, _target in _ALIAS.items():
    _alias(_name, _target)


def _pvm_import(name, globals=None, locals=None, fromlist=(), level=0):
    resolved = _resolve_name(name, globals, level)
    if resolved in _ALIAS:
        _record_import(_ALIAS[resolved], True)
        mod = sys.modules.get(resolved)
        if mod is None:
            raise _HOST.DeterministicValidationError("alias module missing: " + resolved)
        return mod
    allowed = _is_allowed(name, globals, level)
    _record_import(resolved or name, allowed)
    if not allowed and not _TRACE_ALLOW_ALL:
        raise _HOST.NonDeterministicError("module not allowed: " + name)
    return _REAL_IMPORT(name, globals, locals, fromlist, level)


builtins.__import__ = _pvm_import


class _PvmImportGuard:
    def find_spec(self, fullname, path=None, target=None):
        if not _is_allowed(fullname):
            if not _TRACE_ALLOW_ALL:
                raise _HOST.NonDeterministicError("module not allowed: " + fullname)
        return None


sys.meta_path.insert(0, _PvmImportGuard())


def _blocked_open(*_args, **_kwargs):
    raise _HOST.DeterministicValidationError(
        "file IO is disabled in deterministic mode"
    )


builtins.open = _blocked_open
try:
    import io as _io
    _io.open = _blocked_open
except Exception:
    pass

if hasattr(builtins, "execfile"):
    builtins.execfile = _blocked_open
"#;

pub(crate) fn install(
    vm: &VirtualMachine,
    options: &DeterminismOptions,
    host_module_name: &str,
) -> PyResult<()> {
    if !options.enabled {
        return Ok(());
    }

    let scope = vm.new_scope_with_builtins();
    let mut whitelist_items = options.stdlib_whitelist.clone();
    if !whitelist_items.iter().any(|item| item == host_module_name) {
        whitelist_items.push(host_module_name.to_owned());
    }
    let whitelist = to_pylist(vm, &whitelist_items);
    scope
        .globals
        .set_item("PVM_WHITELIST", whitelist.into(), vm)?;
    let blacklist = to_pylist(vm, &options.stdlib_blacklist);
    scope
        .globals
        .set_item("PVM_BLACKLIST", blacklist.into(), vm)?;
    let sys_paths = vm.state.config.paths.module_search_paths.clone();
    let sys_paths_list = to_pylist(vm, &sys_paths);
    scope
        .globals
        .set_item("PVM_SYS_PATH", sys_paths_list.into(), vm)?;
    scope.globals.set_item(
        "PVM_HOST_MODULE",
        vm.ctx.new_str(host_module_name).into(),
        vm,
    )?;
    scope.globals.set_item(
        "PVM_TRACE_IMPORTS",
        vm.ctx.new_bool(options.trace_imports).into(),
        vm,
    )?;
    scope.globals.set_item(
        "PVM_TRACE_ALLOW_ALL",
        vm.ctx.new_bool(options.trace_allow_all).into(),
        vm,
    )?;

    let code = vm
        .compile(GUARD_SOURCE, Mode::Exec, "<pvm_guard>".to_owned())
        .map_err(|err| vm.new_syntax_error(&err, Some(GUARD_SOURCE)))?;
    vm.run_code_obj(code, scope)?;
    Ok(())
}

fn to_pylist(vm: &VirtualMachine, items: &[String]) -> PyListRef {
    let entries: Vec<PyObjectRef> = items
        .iter()
        .map(|item| vm.ctx.new_str(item.as_str()).into())
        .collect();
    vm.ctx.new_list(entries)
}
