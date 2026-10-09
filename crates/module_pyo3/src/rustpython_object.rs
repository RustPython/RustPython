//! RustPython objects as seen from CPython.
//!
//! [`RustPythonObject`] is the mirror image of `pyo3.ref`: a CPython object
//! that forwards attribute access, calls, item access, iteration, comparison
//! and the descriptor protocol to the RustPython object it wraps. A RustPython
//! function stored in a CPython class therefore binds to instances like any
//! other function, and a `property` keeps working as a property.
//!
//! As with `weakref.proxy`, callables get a class of their own, so `callable()`
//! answers truthfully; so do data descriptors, whose `__set__` must not be seen
//! on other objects. Number protocols are the same: `__index__`, `__float__`
//! and `__complex__` exist only on the proxy class of an object that has them.

use alloc::ffi::CString;
use core::ffi::c_int;

use pyo3::exceptions::PyBufferError;
use pyo3::ffi;
use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use pyo3::types::{PyDict, PyMemoryView, PyString, PyTuple};
use rustpython_vm::{
    AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::PyStrRef,
    function::{FuncArgs, KwArgs, PySetterValue},
    protocol::{PyBuffer, PyIter, PyIterReturn},
    types::PyComparisonOp,
};

use crate::convert::{str_to_cpython, to_cpython, to_rustpython};
use crate::gil::{DropAttached, run_rustpython, with_rustpython};

/// A RustPython object handed to CPython by reference.
#[cfg_attr(
    feature = "threading",
    pyo3::pyclass(module = "rustpython", name = "object", subclass, frozen)
)]
#[cfg_attr(
    not(feature = "threading"),
    pyo3::pyclass(module = "rustpython", name = "object", subclass, frozen, unsendable)
)]
pub(crate) struct RustPythonObject {
    obj: DropAttached<PyObjectRef>,
}

/// A callable RustPython object handed to CPython by reference.
#[cfg_attr(
    feature = "threading",
    pyo3::pyclass(module = "rustpython", name = "callable", extends = RustPythonObject, frozen)
)]
#[cfg_attr(
    not(feature = "threading"),
    pyo3::pyclass(
        module = "rustpython",
        name = "callable",
        extends = RustPythonObject,
        frozen,
        unsendable
    )
)]
pub(crate) struct RustPythonCallable;

/// A RustPython data descriptor handed to CPython by reference.
#[cfg_attr(
    feature = "threading",
    pyo3::pyclass(
        module = "rustpython",
        name = "data_descriptor",
        extends = RustPythonObject,
        frozen
    )
)]
#[cfg_attr(
    not(feature = "threading"),
    pyo3::pyclass(
        module = "rustpython",
        name = "data_descriptor",
        extends = RustPythonObject,
        frozen,
        unsendable
    )
)]
pub(crate) struct RustPythonDataDescriptor;

struct ProxyKind {
    callable: bool,
    data_descriptor: bool,
    index: bool,
    float: bool,
    complex: bool,
}

fn proxy_kind(obj: &PyObject, vm: &VirtualMachine) -> ProxyKind {
    let number = &obj.class().slots.as_number;
    ProxyKind {
        callable: obj.is_callable(),
        data_descriptor: obj.class().slots.descr_set.load().is_some(),
        index: number.index.load().is_some(),
        float: number.float.load().is_some(),
        complex: obj
            .class()
            .get_attr(vm.ctx.intern_str("__complex__"))
            .is_some(),
    }
}

/// Whether `obj` supports `__index__`, `__float__` or `__complex__`.
pub(crate) fn has_number_protocol(obj: &PyObject, vm: &VirtualMachine) -> bool {
    let kind = proxy_kind(obj, vm);
    kind.index || kind.float || kind.complex
}

impl RustPythonObject {
    /// Wrap `obj` in the proxy class matching what it supports.
    pub(crate) fn wrap<'py>(
        py: Python<'py>,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> Bound<'py, PyAny> {
        let kind = proxy_kind(&obj, vm);
        let base = Self {
            obj: DropAttached::new(obj),
        };
        let proxy = if kind.callable {
            Bound::new(py, (RustPythonCallable, base)).map(Bound::into_any)
        } else if kind.data_descriptor {
            Bound::new(py, (RustPythonDataDescriptor, base)).map(Bound::into_any)
        } else {
            number_proxy(py, base, kind)
        };
        proxy.expect("allocating a rustpython.object proxy failed")
    }

    fn nb_index<'py>(&self, py: Python<'py>) -> pyo3::PyResult<Bound<'py, PyAny>> {
        self.run_for_cpython(py, NO_ARGS, |obj, (), vm| match obj.number().index(vm) {
            Some(result) => result.map(Into::into),
            None => Err(vm.new_type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                obj.class().name()
            ))),
        })
    }

    fn nb_float<'py>(&self, py: Python<'py>) -> pyo3::PyResult<Bound<'py, PyAny>> {
        self.run_for_cpython(py, NO_ARGS, |obj, (), vm| match obj.number().float(vm) {
            Some(result) => result.map(Into::into),
            None => {
                Err(vm.new_type_error(format!("must be real number, not {}", obj.class().name())))
            }
        })
    }

    fn nb_complex<'py>(&self, py: Python<'py>) -> pyo3::PyResult<Bound<'py, PyAny>> {
        self.run_for_cpython(py, NO_ARGS, |obj, (), vm| {
            obj.get_attr("__complex__", vm)?.call((), vm)
        })
    }

    /// Run `op` on the wrapped object with the arguments `args` converts.
    /// Only `op` runs with the GIL released.
    fn run<A, R>(
        &self,
        py: Python<'_>,
        args: impl FnOnce(&VirtualMachine) -> PyResult<A>,
        op: impl FnOnce(&PyObject, A, &VirtualMachine) -> PyResult<R>,
    ) -> pyo3::PyResult<R> {
        with_rustpython(py, |vm| {
            let args = args(vm)?;
            run_rustpython(py, || op(&self.obj, args, vm))
        })
    }

    /// [`Self::run`] for an operation whose result goes back to CPython.
    fn run_for_cpython<'py, A>(
        &self,
        py: Python<'py>,
        args: impl FnOnce(&VirtualMachine) -> PyResult<A>,
        op: impl FnOnce(&PyObject, A, &VirtualMachine) -> PyResult,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        with_rustpython(py, |vm| {
            let args = args(vm)?;
            let result = run_rustpython(py, || op(&self.obj, args, vm))?;
            to_cpython(py, &result, vm)
        })
    }
}

/// The RustPython object behind `obj`, if `obj` is a [`RustPythonObject`].
pub(crate) fn unwrap_rustpython_object(obj: &Bound<'_, PyAny>) -> Option<PyObjectRef> {
    obj.cast::<RustPythonObject>()
        .ok()
        .map(|proxy| PyObjectRef::clone(&proxy.borrow().obj))
}

/// Arguments of an operation that takes none.
const NO_ARGS: fn(&VirtualMachine) -> PyResult<()> = |_| Ok(());

fn name_to_rustpython(name: &Bound<'_, PyAny>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
    to_rustpython(name, vm)?.try_into_value(vm)
}

fn args_to_rustpython(
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
    vm: &VirtualMachine,
) -> PyResult<FuncArgs> {
    let args = args
        .iter()
        .map(|arg| to_rustpython(&arg, vm))
        .collect::<PyResult<Vec<_>>>()?;
    let kwargs = kwargs
        .into_iter()
        .flatten()
        .map(|(key, value)| {
            let key = to_rustpython(&key, vm)?;
            let key = key
                .downcast_ref::<rustpython_vm::builtins::PyStr>()
                .ok_or_else(|| vm.new_type_error("keywords must be strings"))?
                .as_wtf8()
                .to_owned();
            Ok((key, to_rustpython(&value, vm)?))
        })
        .collect::<PyResult<KwArgs>>()?;
    Ok(FuncArgs::new(args, kwargs))
}

#[pyo3::pymethods]
impl RustPythonObject {
    fn __getattr__<'py>(&self, name: &Bound<'py, PyAny>) -> pyo3::PyResult<Bound<'py, PyAny>> {
        self.run_for_cpython(
            name.py(),
            |vm| name_to_rustpython(name, vm),
            |obj, name, vm| obj.get_attr(&name, vm),
        )
    }

    fn __setattr__(&self, name: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> pyo3::PyResult<()> {
        self.run(
            name.py(),
            |vm| Ok((name_to_rustpython(name, vm)?, to_rustpython(value, vm)?)),
            |obj, (name, value), vm| obj.set_attr(&name, value, vm),
        )
    }

    fn __delattr__(&self, name: &Bound<'_, PyAny>) -> pyo3::PyResult<()> {
        self.run(
            name.py(),
            |vm| name_to_rustpython(name, vm),
            |obj, name, vm| obj.del_attr(&name, vm),
        )
    }

    fn __repr__<'py>(&self, py: Python<'py>) -> pyo3::PyResult<Bound<'py, PyString>> {
        let repr = self.run(py, NO_ARGS, |obj, (), vm| obj.repr(vm))?;
        str_to_cpython(py, repr.as_wtf8())
    }

    fn __str__<'py>(&self, py: Python<'py>) -> pyo3::PyResult<Bound<'py, PyString>> {
        let s = self.run(py, NO_ARGS, |obj, (), vm| obj.str(vm))?;
        str_to_cpython(py, s.as_wtf8())
    }

    fn __len__(&self, py: Python<'_>) -> pyo3::PyResult<usize> {
        self.run(py, NO_ARGS, |obj, (), vm| obj.length(vm))
    }

    fn __getitem__<'py>(&self, key: &Bound<'py, PyAny>) -> pyo3::PyResult<Bound<'py, PyAny>> {
        self.run_for_cpython(
            key.py(),
            |vm| to_rustpython(key, vm),
            |obj, key, vm| obj.get_item(&*key, vm),
        )
    }

    fn __setitem__(&self, key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> pyo3::PyResult<()> {
        self.run(
            key.py(),
            |vm| Ok((to_rustpython(key, vm)?, to_rustpython(value, vm)?)),
            |obj, (key, value), vm| obj.set_item(&*key, value, vm),
        )
    }

    fn __delitem__(&self, key: &Bound<'_, PyAny>) -> pyo3::PyResult<()> {
        self.run(
            key.py(),
            |vm| to_rustpython(key, vm),
            |obj, key, vm| obj.del_item(&*key, vm),
        )
    }

    fn __contains__(&self, item: &Bound<'_, PyAny>) -> pyo3::PyResult<bool> {
        self.run(
            item.py(),
            |vm| to_rustpython(item, vm),
            |obj, item, vm| vm._contains(obj, &item),
        )
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> pyo3::PyResult<Bound<'py, PyAny>> {
        self.run_for_cpython(py, NO_ARGS, |obj, (), vm| obj.get_iter(vm).map(Into::into))
    }

    fn __next__<'py>(&self, py: Python<'py>) -> pyo3::PyResult<Option<Bound<'py, PyAny>>> {
        with_rustpython(py, |vm| {
            match run_rustpython(py, || PyIter::new(&**self.obj).next(vm))? {
                PyIterReturn::Return(value) => to_cpython(py, &value, vm).map(Some),
                PyIterReturn::StopIteration(_) => Ok(None),
            }
        })
    }

    fn __bool__(&self, py: Python<'_>) -> pyo3::PyResult<bool> {
        self.run(py, NO_ARGS, |obj, (), vm| obj.to_owned().is_true(vm))
    }

    fn __hash__(&self, py: Python<'_>) -> pyo3::PyResult<isize> {
        self.run(py, NO_ARGS, |obj, (), vm| {
            obj.hash(vm).map(|hash| hash as isize)
        })
    }

    fn __richcmp__<'py>(
        &self,
        other: &Bound<'py, PyAny>,
        op: CompareOp,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        let op = match op {
            CompareOp::Lt => PyComparisonOp::Lt,
            CompareOp::Le => PyComparisonOp::Le,
            CompareOp::Eq => PyComparisonOp::Eq,
            CompareOp::Ne => PyComparisonOp::Ne,
            CompareOp::Gt => PyComparisonOp::Gt,
            CompareOp::Ge => PyComparisonOp::Ge,
        };
        self.run_for_cpython(
            other.py(),
            |vm| to_rustpython(other, vm),
            |obj, other, vm| obj.to_owned().rich_compare(other, op, vm),
        )
    }

    fn __get__<'py>(
        slf: &Bound<'py, Self>,
        instance: Option<&Bound<'py, PyAny>>,
        owner: Option<&Bound<'py, PyAny>>,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        let this = slf.borrow();
        let Some(descr_get) = this.obj.class().slots.descr_get.load() else {
            return Ok(slf.clone().into_any());
        };
        this.run_for_cpython(
            slf.py(),
            |vm| {
                let instance = instance
                    .filter(|instance| !instance.is_none())
                    .map(|instance| to_rustpython(instance, vm))
                    .transpose()?;
                let owner = owner.map(|owner| to_rustpython(owner, vm)).transpose()?;
                Ok((instance, owner))
            },
            |obj, (instance, owner), vm| descr_get(obj, instance.as_deref(), owner.as_deref(), vm),
        )
    }
}

#[pyo3::pymethods]
impl RustPythonCallable {
    #[pyo3(signature = (*args, **kwargs))]
    fn __call__<'py>(
        slf: &Bound<'py, Self>,
        args: &Bound<'py, PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        slf.as_super().borrow().run_for_cpython(
            slf.py(),
            |vm| args_to_rustpython(args, kwargs, vm),
            |obj, args, vm| obj.call_with_args(args, vm),
        )
    }
}

#[pyo3::pymethods]
impl RustPythonDataDescriptor {
    fn __set__(
        slf: &Bound<'_, Self>,
        instance: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> pyo3::PyResult<()> {
        let value = Some(value);
        descr_set(slf, instance, value)
    }

    fn __delete__(slf: &Bound<'_, Self>, instance: &Bound<'_, PyAny>) -> pyo3::PyResult<()> {
        descr_set(slf, instance, None)
    }
}

fn descr_set(
    slf: &Bound<'_, RustPythonDataDescriptor>,
    instance: &Bound<'_, PyAny>,
    value: Option<&Bound<'_, PyAny>>,
) -> pyo3::PyResult<()> {
    slf.as_super().borrow().run(
        slf.py(),
        |vm| {
            let value = match value {
                Some(value) => PySetterValue::Assign(to_rustpython(value, vm)?),
                None => PySetterValue::Delete,
            };
            Ok((to_rustpython(instance, vm)?, value))
        },
        |obj, (instance, value), vm| {
            let descr_set = obj
                .class()
                .slots
                .descr_set
                .load()
                .expect("data descriptor lost its __set__");
            descr_set(obj, instance, value, vm)
        },
    )
}

fn number_proxy(
    py: Python<'_>,
    base: RustPythonObject,
    kind: ProxyKind,
) -> pyo3::PyResult<Bound<'_, PyAny>> {
    match (kind.index, kind.float, kind.complex) {
        (false, false, false) => Bound::new(py, base).map(Bound::into_any),
        (true, false, false) => Bound::new(py, (RustPythonIndex, base)).map(Bound::into_any),
        (false, true, false) => Bound::new(py, (RustPythonFloat, base)).map(Bound::into_any),
        (false, false, true) => Bound::new(py, (RustPythonComplex, base)).map(Bound::into_any),
        (true, true, false) => Bound::new(py, (RustPythonIndexFloat, base)).map(Bound::into_any),
        (true, false, true) => Bound::new(py, (RustPythonIndexComplex, base)).map(Bound::into_any),
        (false, true, true) => Bound::new(py, (RustPythonFloatComplex, base)).map(Bound::into_any),
        (true, true, true) => Bound::new(py, (RustPythonNumber, base)).map(Bound::into_any),
    }
}

macro_rules! number_proxy_class {
    ($struct:ident, $name:literal, $($method:ident => $body:ident),+ $(,)?) => {
        #[cfg_attr(
            feature = "threading",
            pyo3::pyclass(
                module = "rustpython",
                name = $name,
                extends = RustPythonObject,
                frozen
            )
        )]
        #[cfg_attr(
            not(feature = "threading"),
            pyo3::pyclass(
                module = "rustpython",
                name = $name,
                extends = RustPythonObject,
                frozen,
                unsendable
            )
        )]
        struct $struct;

        #[pyo3::pymethods]
        impl $struct {
            $(
                fn $method<'py>(
                    slf: &Bound<'py, Self>,
                ) -> pyo3::PyResult<Bound<'py, PyAny>> {
                    slf.as_super().borrow().$body(slf.py())
                }
            )+
        }
    };
}

number_proxy_class!(RustPythonIndex, "index", __index__ => nb_index);
number_proxy_class!(RustPythonFloat, "float", __float__ => nb_float);
number_proxy_class!(RustPythonComplex, "complex", __complex__ => nb_complex);
number_proxy_class!(
    RustPythonIndexFloat,
    "index_float",
    __index__ => nb_index,
    __float__ => nb_float
);
number_proxy_class!(
    RustPythonIndexComplex,
    "index_complex",
    __index__ => nb_index,
    __complex__ => nb_complex
);
number_proxy_class!(
    RustPythonFloatComplex,
    "float_complex",
    __float__ => nb_float,
    __complex__ => nb_complex
);
number_proxy_class!(
    RustPythonNumber,
    "number",
    __index__ => nb_index,
    __float__ => nb_float,
    __complex__ => nb_complex
);

/// Exports the memory of a RustPython buffer to CPython.
///
/// The exporter keeps the RustPython export alive, and CPython keeps the
/// exporter alive for as long as a view of it exists, so the memory stays put.
#[cfg_attr(
    feature = "threading",
    pyo3::pyclass(module = "rustpython", name = "buffer", frozen)
)]
#[cfg_attr(
    not(feature = "threading"),
    pyo3::pyclass(module = "rustpython", name = "buffer", frozen, unsendable)
)]
pub(crate) struct RustPythonBuffer {
    _owner: DropAttached<PyObjectRef>,
    buffer: DropAttached<PyBuffer>,
    format: CString,
    shape: Vec<ffi::Py_ssize_t>,
    strides: Vec<ffi::Py_ssize_t>,
}

impl RustPythonBuffer {
    /// A CPython `memoryview` of the contiguous `buffer` exported by `owner`.
    pub(crate) fn memoryview(
        py: Python<'_>,
        owner: PyObjectRef,
        buffer: PyBuffer,
    ) -> pyo3::PyResult<Bound<'_, PyAny>> {
        let desc = &buffer.desc;
        let format = CString::new(desc.format.as_bytes())
            .map_err(|_| PyBufferError::new_err("buffer format contains a NUL character"))?;
        let shape = desc.shape().to_vec();
        let strides = desc.strides().to_vec();
        let exporter = Bound::new(
            py,
            Self {
                _owner: DropAttached::new(owner),
                buffer: DropAttached::new(buffer),
                format,
                shape,
                strides,
            },
        )?;
        Ok(PyMemoryView::from(exporter.as_any())?.into_any())
    }
}

#[pyo3::pymethods]
impl RustPythonBuffer {
    unsafe fn __getbuffer__(
        slf: Bound<'_, Self>,
        view: *mut ffi::Py_buffer,
        flags: c_int,
    ) -> pyo3::PyResult<()> {
        let this = slf.borrow();
        let desc = &this.buffer.desc;
        if flags & ffi::PyBUF_WRITABLE == ffi::PyBUF_WRITABLE && desc.readonly {
            return Err(PyBufferError::new_err("buffer is read-only"));
        }
        let data = this
            .buffer
            .as_contiguous()
            .ok_or_else(|| PyBufferError::new_err("buffer is not contiguous"))?;
        let ptr = data.as_ptr();
        drop(data);

        // SAFETY: `view` is the caller's Py_buffer to fill. Every pointer stored
        // in it points into `this`, which the view keeps alive through `obj`.
        unsafe {
            (*view).obj = slf.clone().into_any().into_ptr();
            (*view).buf = ptr as *mut core::ffi::c_void;
            (*view).len = desc.len as isize;
            (*view).readonly = c_int::from(desc.readonly);
            (*view).itemsize = desc.itemsize as isize;
            (*view).format = if flags & ffi::PyBUF_FORMAT == ffi::PyBUF_FORMAT {
                this.format.as_ptr().cast_mut()
            } else {
                core::ptr::null_mut()
            };
            (*view).ndim = this.shape.len() as c_int;
            (*view).shape = if flags & ffi::PyBUF_ND == ffi::PyBUF_ND {
                this.shape.as_ptr().cast_mut()
            } else {
                core::ptr::null_mut()
            };
            (*view).strides = if flags & ffi::PyBUF_STRIDES == ffi::PyBUF_STRIDES {
                this.strides.as_ptr().cast_mut()
            } else {
                core::ptr::null_mut()
            };
            (*view).suboffsets = core::ptr::null_mut();
            (*view).internal = core::ptr::null_mut();
        }
        Ok(())
    }

    unsafe fn __releasebuffer__(&self, _view: *mut ffi::Py_buffer) {}
}
