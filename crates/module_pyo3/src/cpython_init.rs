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

use core::ffi::CStr;
use std::sync::Once;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAnyMethods, PyCFunction, PyDict, PyTuple};
use rustpython_vm::{
    AsObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::{PyTuple as RustPyTuple, PyType},
    types::{AtomicPyTypeFlags, PyTypeFlags, PyTypeSlots},
};

use crate::_pyo3::module_state;
use crate::convert::to_rustpython;
use crate::errors::cpython_exc_to_rustpython;
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
