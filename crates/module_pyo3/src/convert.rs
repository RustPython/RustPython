//! Moving objects between the two interpreters.
//!
//! Immutable built-in values — `None`, `bool`, `int`, `float`, `complex`,
//! `str`, `bytes` and tuples of them — are copied in both directions: a copy
//! cannot be told apart from the original. A class becomes the class the other
//! side knows it by (see [`crate::pyo3_type`]).
//!
//! Any other CPython object reaches RustPython as an instance of the wrapper
//! of its class, or as a `pyo3.proxy` when that class cannot carry the proxy
//! payload. Either way its identity and mutations are preserved.
//!
//! RustPython objects going to CPython are handled by what CPython code
//! expects of them:
//! - `list`, `dict`, `set`, `frozenset` and `bytearray` are copied, because
//!   extension modules routinely require the exact built-in types;
//! - other buffers are exposed through a `memoryview` sharing their memory;
//! - callables and descriptors become `rustpython.object` proxies, which call
//!   back into RustPython;
//! - anything else is pickled and unpickled in CPython when possible (so a
//!   `datetime` arrives as a CPython `datetime`), and proxied otherwise.

use malachite_bigint::BigInt;
use num_traits::ToPrimitive;
use pyo3::prelude::*;
use pyo3::types::{
    PyBool, PyByteArray, PyBytes, PyComplex, PyDict, PyFloat, PyFrozenSet, PyInt, PyList,
    PyMemoryView, PySet, PyString, PyTuple, PyType as CPythonType,
};
use rustpython_vm::{
    AsObject, PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, VirtualMachine,
    builtins::{
        PyByteArray as RustPyByteArray, PyBytes as RustPyBytes, PyComplex as RustPyComplex,
        PyDict as RustPyDict, PyFloat as RustPyFloat, PyFrozenSet as RustPyFrozenSet,
        PyInt as RustPyInt, PyList as RustPyList, PySet as RustPySet, PyStr,
        PyTuple as RustPyTuple, PyType,
    },
    common::wtf8::{Wtf8, Wtf8Buf},
    function::{FuncArgs, KwArgs},
    protocol::PyBuffer,
};

use crate::errors::Pyo3ResultExt;
use crate::gil::{run_cpython, run_rustpython};
use crate::pyo3_ref::{Pyo3Ref, memoryview_from_cpython};
use crate::pyo3_type::{instance_to_rustpython, type_to_cpython, wrap_cpython_type};
use crate::rustpython_object::{RustPythonBuffer, RustPythonObject, unwrap_rustpython_object};

/// Convert a RustPython object for use in CPython.
pub(crate) fn to_cpython<'py>(
    py: Python<'py>,
    obj: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<Bound<'py, PyAny>> {
    if let Some(proxy) = obj.downcast_ref::<Pyo3Ref>() {
        return Ok(proxy.py_obj.bind(py).clone());
    }
    if let Some(ty) = obj.downcast_ref::<PyType>() {
        return type_to_cpython(py, ty, vm);
    }
    // Containers are copied recursively, and may contain themselves.
    if let Some(value) = vm.with_recursion(" while converting to CPython", || {
        value_to_cpython(py, obj, vm)
    })? {
        return Ok(value);
    }
    if let Ok(buffer) = PyBuffer::try_from_borrowed_object(vm, obj)
        && buffer.desc.is_contiguous()
    {
        return RustPythonBuffer::memoryview(py, obj.to_owned(), buffer).to_pyresult(vm);
    }
    if obj.is_callable() || obj.class().slots.descr_get.load().is_some() {
        return Ok(RustPythonObject::wrap(py, obj.to_owned()));
    }
    Ok(
        pickle_to_cpython(py, obj, vm)
            .unwrap_or_else(|| RustPythonObject::wrap(py, obj.to_owned())),
    )
}

/// Convert a CPython object for use in RustPython.
pub(crate) fn to_rustpython(obj: &Bound<'_, PyAny>, vm: &VirtualMachine) -> PyResult {
    if let Some(original) = unwrap_rustpython_object(obj) {
        return Ok(original);
    }
    if let Some(value) = value_to_rustpython(obj, vm)? {
        return Ok(value);
    }
    if let Ok(ty) = obj.cast::<CPythonType>() {
        return wrap_cpython_type(ty, vm).map(Into::into);
    }
    // A memoryview has to be one: `bytes` and the buffer slot require a real
    // memoryview back from `__buffer__`, and this class cannot carry the
    // proxy payload.
    if obj.cast::<PyMemoryView>().is_ok() {
        return memoryview_from_cpython(obj, vm);
    }
    instance_to_rustpython(obj, vm)
}

/// Call a CPython callable with RustPython arguments.
pub(crate) fn call_cpython<'py>(
    callable: &Bound<'py, PyAny>,
    args: FuncArgs,
    vm: &VirtualMachine,
) -> PyResult<Bound<'py, PyAny>> {
    let py = callable.py();
    let positional = items_to_cpython(py, &args.args, vm)?;
    call_cpython_with(callable, positional, &args.kwargs, vm)
}

/// Call a CPython callable with converted positional arguments.
pub(crate) fn call_cpython_with<'py>(
    callable: &Bound<'py, PyAny>,
    positional: Vec<Bound<'py, PyAny>>,
    kwargs: &KwArgs,
    vm: &VirtualMachine,
) -> PyResult<Bound<'py, PyAny>> {
    let py = callable.py();
    let positional = PyTuple::new(py, positional).to_pyresult(vm)?;
    let keywords = PyDict::new(py);
    for (name, value) in kwargs {
        keywords
            .set_item(
                str_to_cpython(py, name).to_pyresult(vm)?,
                to_cpython(py, value, vm)?,
            )
            .to_pyresult(vm)?;
    }
    run_cpython(vm, || callable.call(positional, Some(&keywords)))
}

/// Copy an immutable built-in value, or a container CPython code expects to
/// receive as the exact built-in type.
fn value_to_cpython<'py>(
    py: Python<'py>,
    obj: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let ctx = &vm.ctx;
    let value = if vm.is_none(obj) {
        py.None().into_bound(py)
    } else if obj.is(&ctx.ellipsis) {
        py.Ellipsis().into_bound(py)
    } else if obj.is(&ctx.not_implemented) {
        py.NotImplemented().into_bound(py)
    } else if obj.class().is(ctx.types.bool_type) {
        PyBool::new(py, obj.is(&ctx.true_value))
            .to_owned()
            .into_any()
    } else if let Some(int) = obj.downcast_ref_if_exact::<RustPyInt>(vm) {
        int_to_cpython(py, int.as_bigint()).to_pyresult(vm)?
    } else if let Some(float) = obj.downcast_ref_if_exact::<RustPyFloat>(vm) {
        PyFloat::new(py, float.to_f64()).into_any()
    } else if let Some(complex) = obj.downcast_ref_if_exact::<RustPyComplex>(vm) {
        let value = complex.to_complex();
        PyComplex::from_doubles(py, value.re, value.im).into_any()
    } else if let Some(s) = obj.downcast_ref_if_exact::<PyStr>(vm) {
        str_to_cpython(py, s.as_wtf8()).to_pyresult(vm)?.into_any()
    } else if let Some(bytes) = obj.downcast_ref_if_exact::<RustPyBytes>(vm) {
        PyBytes::new(py, bytes.as_bytes()).into_any()
    } else if let Some(bytearray) = obj.downcast_ref_if_exact::<RustPyByteArray>(vm) {
        PyByteArray::new(py, &bytearray.borrow_buf()).into_any()
    } else if let Some(tuple) = obj.downcast_ref_if_exact::<RustPyTuple>(vm) {
        let items = items_to_cpython(py, tuple.as_slice(), vm)?;
        PyTuple::new(py, items).to_pyresult(vm)?.into_any()
    } else if let Some(list) = obj.downcast_ref_if_exact::<RustPyList>(vm) {
        let items = list.borrow_vec().to_vec();
        let items = items_to_cpython(py, &items, vm)?;
        PyList::new(py, items).to_pyresult(vm)?.into_any()
    } else if let Some(set) = obj.downcast_ref_if_exact::<RustPySet>(vm) {
        let items = items_to_cpython(py, &set.elements(), vm)?;
        PySet::new(py, items).to_pyresult(vm)?.into_any()
    } else if let Some(set) = obj.downcast_ref_if_exact::<RustPyFrozenSet>(vm) {
        let items = items_to_cpython(py, &set.elements(), vm)?;
        PyFrozenSet::new(py, items).to_pyresult(vm)?.into_any()
    } else if let Some(dict) = obj.downcast_ref_if_exact::<RustPyDict>(vm) {
        let converted = PyDict::new(py);
        for (key, value) in dict.items_vec() {
            converted
                .set_item(to_cpython(py, &key, vm)?, to_cpython(py, &value, vm)?)
                .to_pyresult(vm)?;
        }
        converted.into_any()
    } else {
        return Ok(None);
    };
    Ok(Some(value))
}

fn items_to_cpython<'py>(
    py: Python<'py>,
    items: &[PyObjectRef],
    vm: &VirtualMachine,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    items.iter().map(|item| to_cpython(py, item, vm)).collect()
}

fn value_to_rustpython(
    obj: &Bound<'_, PyAny>,
    vm: &VirtualMachine,
) -> PyResult<Option<PyObjectRef>> {
    let py = obj.py();
    let ctx = &vm.ctx;
    let value: PyObjectRef = if obj.is_none() {
        ctx.none()
    } else if obj.is(py.Ellipsis()) {
        ctx.ellipsis.clone().into()
    } else if obj.is(py.NotImplemented()) {
        ctx.not_implemented()
    } else if let Ok(b) = obj.cast::<PyBool>() {
        ctx.new_bool(b.is_true()).into()
    } else if obj.is_exact_instance_of::<PyInt>() {
        ctx.new_bigint(&int_to_rustpython(obj).to_pyresult(vm)?)
            .into()
    } else if let Ok(float) = obj.cast_exact::<PyFloat>() {
        ctx.new_float(float.value()).into()
    } else if let Ok(complex) = obj.cast_exact::<PyComplex>() {
        ctx.new_complex(num_complex::Complex64::new(complex.real(), complex.imag()))
            .into()
    } else if let Ok(s) = obj.cast_exact::<PyString>() {
        ctx.new_str(str_to_rustpython(s).to_pyresult(vm)?).into()
    } else if let Ok(bytes) = obj.cast_exact::<PyBytes>() {
        ctx.new_bytes(bytes.as_bytes().to_vec()).into()
    } else if let Ok(tuple) = obj.cast_exact::<PyTuple>() {
        let items = tuple
            .iter()
            .map(|item| to_rustpython(&item, vm))
            .collect::<PyResult<Vec<_>>>()?;
        ctx.new_tuple(items).into()
    } else {
        return Ok(None);
    };
    Ok(Some(value))
}

fn int_to_cpython<'py>(py: Python<'py>, value: &BigInt) -> pyo3::PyResult<Bound<'py, PyAny>> {
    if let Some(value) = value.to_i64() {
        return Ok(value.into_pyobject(py)?.into_any());
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item(pyo3::intern!(py, "signed"), true)?;
    py.get_type::<PyInt>().call_method(
        pyo3::intern!(py, "from_bytes"),
        (PyBytes::new(py, &value.to_signed_bytes_le()), "little"),
        Some(&kwargs),
    )
}

fn int_to_rustpython(obj: &Bound<'_, PyAny>) -> pyo3::PyResult<BigInt> {
    if let Ok(value) = obj.extract::<i64>() {
        return Ok(value.into());
    }
    let py = obj.py();
    let bits: usize = obj
        .call_method0(pyo3::intern!(py, "bit_length"))?
        .extract()?;
    let kwargs = PyDict::new(py);
    kwargs.set_item(pyo3::intern!(py, "signed"), true)?;
    let bytes = obj.call_method(
        pyo3::intern!(py, "to_bytes"),
        (bits / 8 + 1, "little"),
        Some(&kwargs),
    )?;
    Ok(BigInt::from_signed_bytes_le(
        bytes.cast::<PyBytes>()?.as_bytes(),
    ))
}

/// Surrogates, which both interpreters allow in `str`, survive the trip.
pub(crate) fn str_to_cpython<'py>(
    py: Python<'py>,
    s: &Wtf8,
) -> pyo3::PyResult<Bound<'py, PyString>> {
    match s.as_str() {
        Ok(s) => Ok(PyString::new(py, s)),
        Err(_) => Ok(PyBytes::new(py, s.as_bytes())
            .call_method1(pyo3::intern!(py, "decode"), ("utf-8", "surrogatepass"))?
            .cast_into::<PyString>()?),
    }
}

fn str_to_rustpython(s: &Bound<'_, PyString>) -> pyo3::PyResult<Wtf8Buf> {
    if let Ok(s) = s.to_str() {
        return Ok(s.into());
    }
    let py = s.py();
    let bytes = s.call_method1(pyo3::intern!(py, "encode"), ("utf-8", "surrogatepass"))?;
    let bytes = bytes.cast::<PyBytes>()?.as_bytes().to_vec();
    Ok(Wtf8Buf::from_bytes(bytes)
        .unwrap_or_else(|bytes| String::from_utf8_lossy(&bytes).into_owned().into()))
}

/// Hand `obj` over by value, if pickle can carry it to CPython.
fn pickle_to_cpython<'py>(
    py: Python<'py>,
    obj: &PyObject,
    vm: &VirtualMachine,
) -> Option<Bound<'py, PyAny>> {
    let data = run_rustpython(py, || {
        vm.import("pickle", 0)
            .and_then(|pickle| pickle.get_attr("dumps", vm))
            .and_then(|dumps| dumps.call((obj.to_owned(),), vm))
    })
    .ok()?;
    let data = data.downcast_ref::<RustPyBytes>()?;
    py.import(pyo3::intern!(py, "pickle"))
        .and_then(|pickle| {
            pickle.call_method1(
                pyo3::intern!(py, "loads"),
                (PyBytes::new(py, data.as_bytes()),),
            )
        })
        .ok()
}
