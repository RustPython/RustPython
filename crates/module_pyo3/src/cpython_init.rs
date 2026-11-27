//! Start the embedded interpreter without reading this process's environment.
//!
//! `PYTHONHOME` and `PYTHONPATH` belong to RustPython. An interpreter started
//! by `Py_InitializeEx` would honor them and then fail to find its own
//! standard library. Configuration here ignores the environment, still imports
//! `site` (so site-packages stay importable) and does not install signal
//! handlers.
//!
//! Exceptions that C code cannot propagate are reported through the embedded
//! interpreter's `sys.unraisablehook`. When this thread is running RustPython,
//! that hook is handed to RustPython's `sys.unraisablehook`.
//!
//! Warnings go the same way. The embedded `warnings.showwarning` is handed to
//! this interpreter's `warnings.showwarning` while a RustPython thread is
//! running, so a recorder installed here observes them. The embedded filters
//! still decide whether `showwarning` is called. If the function here raises,
//! that exception propagates to the embedded caller. With no RustPython thread,
//! or if delivery is already in progress, the embedded function runs.

use core::cell::Cell;
use core::ffi::CStr;
use std::sync::{Once, OnceLock};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAnyMethods, PyCFunction, PyDict, PyTuple, PyType as CPythonType};
use rustpython_vm::{
    AsObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::{PyStr, PyTuple as RustPyTuple, PyType},
    function::{FuncArgs, KwArgs},
    types::{AtomicPyTypeFlags, PyTypeFlags, PyTypeSlots},
};

use crate::_pyo3::module_state;
use crate::convert::to_rustpython;
use crate::errors::{cpython_exc_to_rustpython, matching_builtin_type, rustpython_exc_to_cpython};
use crate::gil::run_rustpython;

/// Initialize CPython on first use and install the unraisable forwarder.
///
/// Must run before the first `Python::attach`. Initialization leaves this
/// thread holding the GIL; it is released here, which is what a later attach
/// expects. The forwarder is installed afterwards, because attaching during
/// initialization would re-enter it.
pub(crate) fn ensure_cpython_initialized() {
    static INIT: Once = Once::new();
    INIT.call_once(initialize_cpython);
    install_unraisable_hook();
    install_warning_hook();
}

fn initialize_cpython() {
    if unsafe { pyo3::ffi::Py_IsInitialized() } != 0 {
        return;
    }
    // SAFETY: the config is initialized by PyConfig_InitPythonConfig before
    // any field is read, and cleared exactly once on every path below.
    let mut config = unsafe { core::mem::zeroed::<pyo3::ffi::PyConfig>() };
    unsafe { pyo3::ffi::PyConfig_InitPythonConfig(&mut config) };
    config.use_environment = 0;
    config.install_signal_handlers = 0;
    config.parse_argv = 0;
    let status = unsafe { pyo3::ffi::Py_InitializeFromConfig(&config) };
    let failed = unsafe { pyo3::ffi::PyStatus_Exception(status) } != 0;
    let message = if failed {
        status_message(status)
    } else {
        String::new()
    };
    unsafe { pyo3::ffi::PyConfig_Clear(&mut config) };
    assert!(!failed, "failed to initialize embedded CPython: {message}");
    // `Py_InitializeFromConfig` leaves the GIL held. Release it before any
    // `Python::attach`, matching `Py_InitializeEx(0)`.
    unsafe { pyo3::ffi::PyEval_SaveThread() };
}

fn status_message(status: pyo3::ffi::PyStatus) -> String {
    if status.err_msg.is_null() {
        return "CPython initialization failed".to_owned();
    }
    // SAFETY: on failure `err_msg` points at a NUL-terminated message, copied
    // before the config that produced the status is cleared.
    unsafe { CStr::from_ptr(status.err_msg) }
        .to_string_lossy()
        .into_owned()
}

fn install_unraisable_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        Python::attach(|py| {
            if let Err(err) = set_unraisable_hook(py) {
                panic!("failed to install the embedded unraisable hook: {err}");
            }
        });
    });
}

fn set_unraisable_hook(py: Python<'_>) -> pyo3::PyResult<()> {
    let hook = PyCFunction::new_closure(py, Some(c"unraisablehook"), None, unraisable_hook)?;
    py.import("sys")?.setattr("unraisablehook", hook)
}

static ORIGINAL_SHOWWARNING: OnceLock<Py<PyAny>> = OnceLock::new();

std::thread_local! {
    static IN_SHOWWARNING: Cell<bool> = const { Cell::new(false) };
}

/// Set while a warning is being delivered, so a nested report uses the
/// embedded function instead of calling back into the same delivery.
struct ShowwarningGuard;

impl ShowwarningGuard {
    fn enter() -> Option<Self> {
        IN_SHOWWARNING.with(|flag| {
            if flag.get() {
                None
            } else {
                flag.set(true);
                Some(Self)
            }
        })
    }
}

impl Drop for ShowwarningGuard {
    fn drop(&mut self) {
        IN_SHOWWARNING.with(|flag| flag.set(false));
    }
}

fn install_warning_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        Python::attach(|py| {
            if let Err(err) = set_warning_hook(py) {
                panic!("failed to install the embedded warning hook: {err}");
            }
        });
    });
}

fn set_warning_hook(py: Python<'_>) -> pyo3::PyResult<()> {
    let warnings = py.import("warnings")?;
    let original = warnings.getattr("showwarning")?;
    let _ = ORIGINAL_SHOWWARNING.set(original.unbind());
    let hook = PyCFunction::new_closure(py, Some(c"showwarning"), None, showwarning_hook)?;
    warnings.setattr("showwarning", hook)
}

fn showwarning_hook(
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> pyo3::PyResult<()> {
    let Some(_guard) = ShowwarningGuard::enter() else {
        return call_original_showwarning(args, kwargs);
    };
    let py = args.py();
    let forwarded = rustpython_vm::vm::thread::try_with_current_vm(|vm| {
        vm.attach_for_callback(|| forward_showwarning(py, args, kwargs, vm))
    });
    match forwarded {
        Some(result) => result,
        None => call_original_showwarning(args, kwargs),
    }
}

fn forward_showwarning(
    py: Python<'_>,
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
    vm: &VirtualMachine,
) -> pyo3::PyResult<()> {
    let call_args =
        showwarning_args(args, kwargs, vm).map_err(|err| rustpython_exc_to_cpython(py, err, vm))?;
    run_rustpython(py, || {
        let warnings = vm.import("warnings", 0)?;
        let show = warnings.get_attr("showwarning", vm)?;
        show.call(call_args, vm)?;
        Ok(())
    })
    .map_err(|err| rustpython_exc_to_cpython(py, err, vm))
}

fn showwarning_args(
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
    vm: &VirtualMachine,
) -> PyResult<FuncArgs> {
    let positional = args
        .iter()
        .enumerate()
        .map(|(index, arg)| showwarning_value(index == 1, &arg, vm))
        .collect::<PyResult<Vec<_>>>()?;
    let kwargs = kwargs
        .into_iter()
        .flatten()
        .map(|(key, value)| {
            let key = to_rustpython(&key, vm)?;
            let key = key
                .downcast_ref::<PyStr>()
                .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
            let is_category = key.to_str() == Some("category");
            let value = showwarning_value(is_category, &value, vm)?;
            Ok((key.as_wtf8().to_owned(), value))
        })
        .collect::<PyResult<KwArgs>>()?;
    Ok(FuncArgs::new(positional, kwargs))
}

/// `category` is a warning class. Hand this interpreter's class across so a
/// filter or recorder here sees the class it already imported.
fn showwarning_value(category: bool, value: &Bound<'_, PyAny>, vm: &VirtualMachine) -> PyResult {
    if category
        && let Ok(ty) = value.cast::<CPythonType>()
        && let Some(mapped) = matching_builtin_type(ty, vm)
        && mapped.fast_issubclass(vm.ctx.exceptions.warning)
    {
        return Ok(mapped.into());
    }
    to_rustpython(value, vm)
}

fn call_original_showwarning(
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> pyo3::PyResult<()> {
    let Some(original) = ORIGINAL_SHOWWARNING.get() else {
        return Ok(());
    };
    original.bind(args.py()).call(args, kwargs)?;
    Ok(())
}

/// What happened to one unraisable exception.
enum Forwarded {
    /// RustPython's hook ran and returned.
    Reported,
    /// RustPython's hook ran and raised. The original exception was already
    /// delivered, so it must not be reported again.
    HookFailed,
    /// The hook was not called.
    NotReported,
}

fn unraisable_hook(
    args: &Bound<'_, PyTuple>,
    _kwargs: Option<&Bound<'_, PyDict>>,
) -> pyo3::PyResult<()> {
    let Ok(unraisable) = args.get_item(0) else {
        return Ok(());
    };
    let py = unraisable.py();
    let forwarded = rustpython_vm::vm::thread::try_with_current_vm(|vm| {
        vm.attach_for_callback(|| forward_unraisable(py, &unraisable, vm))
    });
    match forwarded {
        Some(Forwarded::Reported | Forwarded::HookFailed) => Ok(()),
        Some(Forwarded::NotReported) | None => call_default_unraisable(py, &unraisable),
    }
}

fn forward_unraisable(
    py: Python<'_>,
    unraisable: &Bound<'_, PyAny>,
    vm: &VirtualMachine,
) -> Forwarded {
    let args = match unraisable_hook_args(unraisable, vm) {
        Ok(args) => args,
        Err(_) => return Forwarded::NotReported,
    };
    let Ok(hook) = vm.sys_module.get_attr("unraisablehook", vm) else {
        return Forwarded::NotReported;
    };
    match run_rustpython(py, || hook.call((args,), vm)) {
        Ok(_) => Forwarded::Reported,
        Err(_) => Forwarded::HookFailed,
    }
}

fn call_default_unraisable(py: Python<'_>, unraisable: &Bound<'_, PyAny>) -> pyo3::PyResult<()> {
    let hook = py
        .import("sys")
        .map_err(|_| PyRuntimeError::new_err("embedded sys module is missing"))?
        .getattr("__unraisablehook__")?;
    hook.call1((unraisable,))?;
    Ok(())
}

/// Build the object RustPython's hook receives.
///
/// The builtin hook reads a 5-sequence. `support.catch_unraisable_exception`
/// stores the object and reads its attributes. A tuple subclass with a dict
/// is both.
fn unraisable_hook_args(unraisable: &Bound<'_, PyAny>, vm: &VirtualMachine) -> PyResult {
    let py = unraisable.py();
    let exc_value = unraisable
        .getattr(pyo3::intern!(py, "exc_value"))
        .map_err(|_| vm.new_type_error("unraisable hook argument has no exc_value"))?;
    let exc_value = cpython_exc_to_rustpython(&exc_value, vm);
    let exc_type: PyObjectRef = exc_value.class().to_owned().into();
    let elements = vec![
        exc_type,
        exc_value.into(),
        unraisable_attr(unraisable, "exc_traceback", vm)?,
        unraisable_attr(unraisable, "err_msg", vm)?,
        unraisable_attr(unraisable, "object", vm)?,
    ];
    let ty = unraisable_args_type(vm)?;
    let payload = RustPyTuple::new_unchecked(elements.clone().into_boxed_slice());
    let obj = payload.into_ref_with_type(vm, ty)?;
    for (name, value) in [
        "exc_type",
        "exc_value",
        "exc_traceback",
        "err_msg",
        "object",
    ]
    .into_iter()
    .zip(elements)
    {
        obj.as_object().set_attr(name, value, vm)?;
    }
    Ok(obj.into())
}

fn unraisable_attr(obj: &Bound<'_, PyAny>, name: &str, vm: &VirtualMachine) -> PyResult {
    match obj.getattr(name) {
        Ok(value) => to_rustpython(&value, vm),
        Err(_) => Ok(vm.ctx.none()),
    }
}

fn unraisable_args_type(vm: &VirtualMachine) -> PyResult<rustpython_vm::builtins::PyTypeRef> {
    let state = module_state(vm)?;
    if let Some(ty) = state.cached_unraisable_args() {
        return Ok(ty);
    }
    let mut slots = PyTypeSlots::default();
    slots.flags = AtomicPyTypeFlags::from_plain(PyTypeFlags::from_slice(&[
        PyTypeFlags::HEAPTYPE,
        PyTypeFlags::BASETYPE,
        PyTypeFlags::HAS_DICT,
        PyTypeFlags::MANAGED_DICT,
    ]));
    let ty = PyType::new_heap(
        "UnraisableHookArgs",
        vec![vm.ctx.types.tuple_type.to_owned()],
        Default::default(),
        slots,
        vm.ctx.types.type_type.to_owned(),
        &vm.ctx,
    )
    .map_err(|err| vm.new_type_error(err))?;
    Ok(state.store_unraisable_args(ty))
}
