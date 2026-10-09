//! Exceptions crossing between the two interpreters.
//!
//! An exception is rebuilt as the nearest built-in exception class of the other
//! side, with its `args` converted, so `except ValueError:` works whichever
//! interpreter raised it. When the class itself has no counterpart (say
//! `decimal.InvalidOperation`) the rebuilt exception carries a note naming the
//! original class. An exception that returns to the interpreter it came from
//! comes back as the very same object.

use pyo3::prelude::*;
use pyo3::types::{PyTuple, PyType as CPythonType};
use rustpython_vm::{
    AsObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::{PyBaseException, PyBaseExceptionRef, PyStr, PyType},
};

use crate::convert::{to_cpython, to_rustpython};
use crate::gil::with_gil;
use crate::pyo3_ref::Pyo3Ref;
use crate::rustpython_object::{RustPythonObject, unwrap_rustpython_object};

/// Attribute of a CPython exception holding the RustPython exception it was
/// made from.
const RUSTPYTHON_ORIGIN_ATTR: &str = "_rustpython_exception";
/// Attribute of a RustPython exception holding the CPython exception it was
/// made from.
const CPYTHON_ORIGIN_ATTR: &str = "_cpython_exception";
/// Attributes of built-in exceptions that are not part of `args`
/// (`ImportError.name`, `AttributeError.obj`...).
const EXTRA_ATTRS: [&str; 4] = ["name", "path", "name_from", "obj"];

pub(crate) trait Pyo3ResultExt<T> {
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult<T>;
}

impl<T> Pyo3ResultExt<T> for pyo3::PyResult<T> {
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult<T> {
        self.map_err(|err| cpython_err_to_rustpython(err, vm))
    }
}

pub(crate) fn cpython_err_to_rustpython(err: PyErr, vm: &VirtualMachine) -> PyBaseExceptionRef {
    with_gil(vm, |py| {
        let value = err.into_value(py).into_bound(py);
        cpython_exc_to_rustpython(value.as_any(), vm)
    })
}

pub(crate) fn cpython_exc_to_rustpython(
    value: &Bound<'_, PyAny>,
    vm: &VirtualMachine,
) -> PyBaseExceptionRef {
    let py = value.py();
    if let Ok(origin) = value.getattr(pyo3::intern!(py, RUSTPYTHON_ORIGIN_ATTR))
        && let Some(obj) = unwrap_rustpython_object(&origin)
        && let Ok(exc) = obj.downcast::<PyBaseException>()
    {
        return exc;
    }

    let cpython_type = value.get_type();
    let (rustpython_type, exact) = cpython_mro_builtin(&cpython_type)
        .and_then(|(name, exact)| {
            let ty = vm
                .builtins
                .get_attr(&vm.ctx.new_str(name.as_str()), vm)
                .ok()?;
            let ty = ty.downcast::<PyType>().ok()?;
            ty.fast_issubclass(vm.ctx.exceptions.base_exception_type)
                .then_some((ty, exact))
        })
        .unwrap_or_else(|| (vm.ctx.exceptions.runtime_error.to_owned(), false));

    let args = value
        .getattr(pyo3::intern!(py, "args"))
        .ok()
        .and_then(|args| args.cast_into::<PyTuple>().ok())
        .and_then(|args| {
            args.iter()
                .map(|arg| to_rustpython(&arg, vm))
                .collect::<PyResult<Vec<_>>>()
                .ok()
        })
        .unwrap_or_else(|| vec![vm.ctx.new_str(value.to_string()).into()]);

    let exc = vm
        .invoke_exception(&rustpython_type, args)
        .unwrap_or_else(|err| err);
    for name in EXTRA_ATTRS {
        if let Ok(value) = value.getattr(name)
            && exc.as_object().get_attr(name, vm).is_ok()
            && let Ok(value) = to_rustpython(&value, vm)
        {
            let _ = exc.as_object().set_attr(name, value, vm);
        }
    }
    let _ = exc.as_object().set_attr(
        CPYTHON_ORIGIN_ATTR,
        Pyo3Ref::new(value.clone().unbind()).into_ref(&vm.ctx),
        vm,
    );
    if !exact {
        let note = format!("raised in CPython as {}", cpython_type_name(&cpython_type));
        let _ = exc.add_note(vm.ctx.new_str(note), vm);
    }
    exc
}

pub(crate) fn rustpython_exc_to_cpython(
    py: Python<'_>,
    exc: PyBaseExceptionRef,
    vm: &VirtualMachine,
) -> PyErr {
    if let Ok(origin) = exc.as_object().get_attr(CPYTHON_ORIGIN_ATTR, vm)
        && let Some(origin) = origin.downcast_ref::<Pyo3Ref>()
    {
        return PyErr::from_value(origin.py_obj.bind(py).clone());
    }

    let rustpython_type = exc.class();
    let (cpython_type, exact) = rustpython_type
        .mro_collect()
        .iter()
        .enumerate()
        .find_map(|(i, cls)| {
            let module = cls.__module__(vm).ok()?;
            if module.downcast_ref::<PyStr>()?.to_str()? != "builtins" {
                return None;
            }
            let ty = py.import("builtins").ok()?.getattr(&*cls.name()).ok()?;
            let ty = ty.cast_into::<CPythonType>().ok()?;
            ty.is_subclass_of::<pyo3::exceptions::PyBaseException>()
                .ok()?
                .then_some((ty, i == 0))
        })
        .unwrap_or_else(|| (py.get_type::<pyo3::exceptions::PyRuntimeError>(), false));

    let instance = exc
        .args()
        .iter()
        .map(|arg| to_cpython(py, arg, vm))
        .collect::<PyResult<Vec<_>>>()
        .ok()
        .and_then(|args| PyTuple::new(py, args).ok())
        .and_then(|args| cpython_type.call1(args).ok())
        .or_else(|| {
            cpython_type
                .call1((exc.as_object().str(vm).ok()?.to_string(),))
                .ok()
        });
    let Some(instance) = instance else {
        return pyo3::exceptions::PyRuntimeError::new_err(
            "failed to convert a RustPython exception to CPython",
        );
    };

    for name in EXTRA_ATTRS {
        if let Ok(value) = exc.as_object().get_attr(name, vm)
            && instance.hasattr(name).unwrap_or(false)
            && let Ok(value) = to_cpython(py, &value, vm)
        {
            let _ = instance.setattr(name, value);
        }
    }

    let origin: PyObjectRef = exc.clone().into();
    let _ = instance.setattr(
        pyo3::intern!(py, RUSTPYTHON_ORIGIN_ATTR),
        RustPythonObject::wrap(py, origin, vm),
    );
    if !exact && let Ok(name) = rustpython_type.fully_qualified_name(vm) {
        let _ = instance.call_method1(
            pyo3::intern!(py, "add_note"),
            (format!("raised in RustPython as {name}"),),
        );
    }
    PyErr::from_value(instance)
}

/// The first class in `ty.__mro__` defined in `builtins`, and whether that is
/// `ty` itself.
fn cpython_mro_builtin(ty: &Bound<'_, CPythonType>) -> Option<(String, bool)> {
    ty.mro().iter().enumerate().find_map(|(i, cls)| {
        let cls = cls.cast_into::<CPythonType>().ok()?;
        (cls.module().ok()?.to_str().ok()? == "builtins")
            .then(|| Some((cls.name().ok()?.to_string(), i == 0)))
            .flatten()
    })
}

fn cpython_type_name(ty: &Bound<'_, CPythonType>) -> String {
    ty.fully_qualified_name()
        .map_or_else(|_| "<unknown>".to_owned(), |name| name.to_string())
}
