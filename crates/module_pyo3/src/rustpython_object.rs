//! RustPython objects as seen from CPython.
//!
//! [`RustPythonObject`] is the mirror image of `pyo3.ref`: a CPython object
//! that forwards attribute access, calls, item access, comparison
//! and the descriptor protocol to the RustPython object it wraps. Iteration
//! is forwarded only when the wrapped class defines it. A RustPython function
//! stored in a CPython class therefore binds to instances like any other
//! function, and a `property` keeps working as a property.
//!
//! As with `weakref.proxy`, callables get a class of their own, so `callable()`
//! answers truthfully; so do data descriptors, whose `__set__` must not be seen
//! on other objects. Number protocols follow the same rule. The proxy class
//! carries exactly the slots the wrapped object's class defines — `__index__`,
//! `__float__`, `__complex__`, unary `-`, `+`, `~` and `abs`, and the binary
//! operators `+ - * / // % ** @ & | ^ << >>` with their reflected forms — and
//! nothing that class lacks. One proxy class is cached for each distinct set
//! of those slots, together with whether the object is callable or a data
//! descriptor, so the number of classes stays bounded as instances come and
//! go. An object that publishes no extra dunder, has no number protocol, and
//! is not iterable still uses the callable class, the data descriptor class,
//! or this base class. An object that has a number protocol, that publishes
//! extra dunders, or whose class defines `__iter__` or `__next__`, keeps
//! `__call__` or `__set__` on that same proxy class when it is callable or a
//! data descriptor. `__iter__` and `__next__` are installed only when those
//! slots exist, so a callable that is not iterable does not look iterable.
//! The proxy is not an instance of the wrapped class, so a registry keyed by
//! that class does not match the proxy.
//!
//! The sequence length slot reads `__len__`. Sequence item access with a
//! negative index adds that length first. Subscripting from Python passes the
//! index through unchanged.
//!
//! A dunder that is not one of those slots is published on the proxy class
//! when the wrapped class defines it as a plain function. The first class in
//! the MRO that defines the name decides: a later base does not put the name
//! back, and `None` or any other non-method opts it out. A name only `object`
//! defines stays unpublished. A name the class itself defines is published
//! even when `object` also has it, including `__reduce__`, `__reduce_ex__`,
//! `__format__`, `__sizeof__`, `__dir__` and `__getstate__`. Lookup on the
//! proxy class finds that function, and a call receives the wrapped object as
//! its first argument. Lookup on an instance binds that object, so the call's
//! other arguments follow it. The name is absent when the class does not
//! define it. A data descriptor is not published: reading it from an instance
//! still runs the descriptor and returns its value. Classes that publish the
//! same names share a proxy class, so the cache stays bounded by classes
//! rather than instances. The name set of one RustPython class is remembered
//! with that class's version tag and the versions of its bases. A modification
//! clears the tag, so the record no longer matches. The record stores no
//! strong reference to the class and the map is capped. The sequence length
//! slot is installed once; waiting for that install releases the GIL.

use alloc::collections::BTreeMap;
use alloc::ffi::CString;
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};
use core::ffi::{c_char, c_int, c_void};
use core::ops::Deref;
use core::sync::atomic::Ordering;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, Once};

use pyo3::sync::{MutexExt, OnceExt};

use pyo3::exceptions::{PyBufferError, PyTypeError};
use pyo3::ffi;
use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use pyo3::type_object::PyTypeInfo;
use pyo3::types::{PyDict, PyMemoryView, PyString, PyTuple, PyType, PyTypeMethods};
use rustpython_vm::{
    AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::PyStrRef,
    function::{FuncArgs, KwArgs, PySetterValue},
    protocol::{
        BufferFlags, PyBuffer, PyIter, PyIterReturn, PyNumberBinaryOp, PyNumberSlots,
        PyNumberTernaryOp, PyNumberUnaryFunc,
    },
    types::PyComparisonOp,
};

use crate::convert::{str_to_cpython, to_cpython, to_rustpython};
use crate::errors::Pyo3ResultExt;
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

/// Set when the wrapped object is callable. Not a number protocol.
const CALL: u64 = 1u64 << 33;
/// Set when the wrapped object is a data descriptor. Not a number protocol.
const DESCR: u64 = 1u64 << 34;
/// Set when the wrapped class defines iteration. Not a number protocol.
const ITER: u64 = 1u64 << 35;
/// Set when the wrapped class defines the next step of an iterator. Not a number protocol.
const NEXT: u64 = 1u64 << 36;
/// Bits that choose a proxy shape without being number protocols.
const SHAPE: u64 = CALL | DESCR | ITER | NEXT;

fn proxy_bits(obj: &PyObject, vm: &VirtualMachine) -> u64 {
    let mut bits = 0;
    for method in NUMBER_METHODS {
        if (method.present)(obj, vm) {
            bits |= method.bit;
        }
    }
    if obj.is_callable() {
        bits |= CALL;
    }
    if obj.class().slots.descr_set.load().is_some() {
        bits |= DESCR;
    }
    if obj.class().slots.iter.load().is_some() {
        bits |= ITER;
    }
    if obj.class().slots.iternext.load().is_some() {
        bits |= NEXT;
    }
    bits
}

/// Whether `obj`'s class defines a number protocol this proxy can forward.
pub(crate) fn has_number_protocol(obj: &PyObject, vm: &VirtualMachine) -> bool {
    (proxy_bits(obj, vm) & !SHAPE) != 0
}

impl RustPythonObject {
    /// Wrap `obj` in the proxy class matching what it supports.
    pub(crate) fn wrap<'py>(
        py: Python<'py>,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        ensure_sequence_length(py);
        let bits = proxy_bits(&obj, vm);
        let specials = cached_published_specials(py, &obj, vm);
        let base = Self {
            obj: DropAttached::new(obj),
        };
        // The base classes have no iteration slots. An iterable object needs
        // its own proxy class; a callable that is not iterable does not.
        if specials.is_empty() && (bits & !SHAPE) == 0 && (bits & (ITER | NEXT)) == 0 {
            if (bits & CALL) != 0 {
                Bound::new(py, (RustPythonCallable, base)).map(Bound::into_any)
            } else if (bits & DESCR) != 0 {
                Bound::new(py, (RustPythonDataDescriptor, base)).map(Bound::into_any)
            } else {
                Bound::new(py, base).map(Bound::into_any)
            }
        } else {
            let ty = proxy_type(py, &ProxyKey { bits, specials })?;
            push_pending(base);
            let _guard = PendingGuard;
            ty.call0()
        }
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

    fn iter_proxy<'py>(&self, py: Python<'py>) -> pyo3::PyResult<Bound<'py, PyAny>> {
        self.run_for_cpython(py, NO_ARGS, |obj, (), vm| obj.get_iter(vm).map(Into::into))
    }

    fn next_proxy<'py>(&self, py: Python<'py>) -> pyo3::PyResult<Bound<'py, PyAny>> {
        with_rustpython(py, |vm| {
            match run_rustpython(py, || PyIter::new(&**self.obj).next(vm))? {
                PyIterReturn::Return(value) => to_cpython(py, &value, vm),
                PyIterReturn::StopIteration(_) => Err(vm.new_stop_iteration(None)),
            }
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

    /// `reflected` selects the right-hand slot. That slot calls
    /// `__r*__(right, left)`, so the reflected call passes `(other, self)`.
    /// A missing slot returns `NotImplemented`.
    fn forward_binary<'py>(
        &self,
        other: &Bound<'py, PyAny>,
        op: PyNumberBinaryOp,
        reflected: bool,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        self.run_for_cpython(
            other.py(),
            |vm| to_rustpython(other, vm),
            |obj, other, vm| {
                let slots = &obj.class().slots.as_number;
                let slot = if reflected {
                    slots.right_binary_op(op)
                } else {
                    slots.left_binary_op(op)
                };
                let Some(slot) = slot else {
                    return Ok(vm.ctx.not_implemented());
                };
                let (left, right) = if reflected {
                    (&*other, obj)
                } else {
                    (obj, &*other)
                };
                slot(left, right, vm)
            },
        )
    }

    fn forward_unary<'py>(
        &self,
        py: Python<'py>,
        slot: fn(&PyNumberSlots) -> Option<PyNumberUnaryFunc>,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        self.run_for_cpython(py, NO_ARGS, |obj, (), vm| {
            let Some(func) = slot(&obj.class().slots.as_number) else {
                return Ok(vm.ctx.not_implemented());
            };
            func(obj.number(), vm)
        })
    }

    /// `args` is `(other,)` or `(other, modulo)`. A missing slot returns
    /// `NotImplemented`. The reflected slot calls `__rpow__(self, other)`.
    fn forward_power<'py>(
        &self,
        args: &Bound<'py, PyTuple>,
        reflected: bool,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        self.run_for_cpython(
            args.py(),
            |vm| {
                let other = to_rustpython(&args.get_item(0).to_pyresult(vm)?, vm)?;
                let modulo = if PyTupleMethods::len(args) > 1 {
                    to_rustpython(&args.get_item(1).to_pyresult(vm)?, vm)?
                } else {
                    vm.ctx.none()
                };
                Ok((other, modulo))
            },
            |obj, (other, modulo), vm| {
                let slots = &obj.class().slots.as_number;
                let slot = if reflected {
                    slots.right_ternary_op(PyNumberTernaryOp::Power)
                } else {
                    slots.left_ternary_op(PyNumberTernaryOp::Power)
                };
                let Some(slot) = slot else {
                    return Ok(vm.ctx.not_implemented());
                };
                let (base, exp) = if reflected {
                    (&*other, obj)
                } else {
                    (obj, &*other)
                };
                slot(base, exp, &modulo, vm)
            },
        )
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
    #[new]
    fn new() -> pyo3::PyResult<Self> {
        take_pending()
            .ok_or_else(|| PyTypeError::new_err("cannot create rustpython.object instances"))
    }

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
        call_object(slf.as_super(), args, kwargs)
    }
}

#[pyo3::pymethods]
impl RustPythonDataDescriptor {
    fn __set__(
        slf: &Bound<'_, Self>,
        instance: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> pyo3::PyResult<()> {
        descr_set(slf.as_super(), instance, Some(value))
    }

    fn __delete__(slf: &Bound<'_, Self>, instance: &Bound<'_, PyAny>) -> pyo3::PyResult<()> {
        descr_set(slf.as_super(), instance, None)
    }
}

fn call_object<'py>(
    slf: &Bound<'py, RustPythonObject>,
    args: &Bound<'py, PyTuple>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> pyo3::PyResult<Bound<'py, PyAny>> {
    slf.borrow().run_for_cpython(
        slf.py(),
        |vm| args_to_rustpython(args, kwargs, vm),
        |obj, args, vm| obj.call_with_args(args, vm),
    )
}

fn descr_set(
    slf: &Bound<'_, RustPythonObject>,
    instance: &Bound<'_, PyAny>,
    value: Option<&Bound<'_, PyAny>>,
) -> pyo3::PyResult<()> {
    slf.borrow().run(
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

std::thread_local! {
    static PENDING_PROXY: RefCell<Vec<Option<RustPythonObject>>> = const { RefCell::new(Vec::new()) };
}

fn push_pending(obj: RustPythonObject) {
    PENDING_PROXY.with(|pending| pending.borrow_mut().push(Some(obj)));
}

fn take_pending() -> Option<RustPythonObject> {
    PENDING_PROXY.with(|pending| pending.borrow_mut().last_mut().and_then(Option::take))
}

fn pop_pending() {
    PENDING_PROXY.with(|pending| {
        pending.borrow_mut().pop();
    });
}

/// Pops the frame [`push_pending`] pushed, including when instantiation fails.
struct PendingGuard;

impl Drop for PendingGuard {
    fn drop(&mut self) {
        pop_pending();
    }
}

/// Number, call, and descriptor bits, plus the dunder names published on top.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ProxyKey {
    bits: u64,
    specials: Arc<[Box<str>]>,
}

static PROXY_TYPES: LazyLock<Mutex<HashMap<ProxyKey, Py<PyType>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn proxy_type<'py>(py: Python<'py>, key: &ProxyKey) -> pyo3::PyResult<Bound<'py, PyType>> {
    if let Some(ty) = lookup_proxy_type(py, key) {
        return Ok(ty);
    }
    let created = create_proxy_type(py, key)?;
    // Hold the lock only while updating the map. Creating the type runs
    // Python and can re-enter this function.
    let stored = {
        let mut types = PROXY_TYPES.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(existing) = types.get(key) {
            existing.clone_ref(py)
        } else {
            let stored = created.unbind();
            types.insert(key.clone(), stored.clone_ref(py));
            stored
        }
    };
    Ok(stored.into_bound(py))
}

fn lookup_proxy_type<'py>(py: Python<'py>, key: &ProxyKey) -> Option<Bound<'py, PyType>> {
    let ty = {
        let types = PROXY_TYPES.lock().unwrap_or_else(|err| err.into_inner());
        types.get(key).map(|ty| ty.clone_ref(py))
    };
    ty.map(|ty| ty.into_bound(py))
}

fn create_proxy_type<'py>(py: Python<'py>, key: &ProxyKey) -> pyo3::PyResult<Bound<'py, PyType>> {
    let type_ctor = py.import("builtins")?.getattr("type")?;
    let name = if key.specials.is_empty() {
        format!("number_{:x}", key.bits)
    } else {
        format!("proxy_{:x}_{}", key.bits, key.specials.len())
    };
    let base = RustPythonObject::type_object(py);
    let ty = type_ctor.call1((name, (base,), PyDict::new(py)))?;
    let ty = ty.cast::<PyType>()?.clone();
    ty.setattr("__module__", "rustpython")?;
    for method in NUMBER_METHODS {
        if (key.bits & method.bit) != 0 {
            install_method(&ty, method.name, &method.def.0)?;
        }
    }
    if (key.bits & CALL) != 0 {
        install_method(&ty, "__call__", &CALL_DEF.0)?;
    }
    if (key.bits & DESCR) != 0 {
        install_method(&ty, "__set__", &SET_DEF.0)?;
        install_method(&ty, "__delete__", &DELETE_DEF.0)?;
    }
    if (key.bits & ITER) != 0 {
        install_method(&ty, "__iter__", &ITER_DEF.0)?;
    }
    if (key.bits & NEXT) != 0 {
        install_method(&ty, "__next__", &NEXT_DEF.0)?;
    }
    // Names installed above live in this class's own dict. A name inherited
    // from `object` does not, and the wrapped class's method has to replace it.
    // SAFETY: `ty` is a ready heap type. `tp_dict` is its own namespace,
    // borrowed from the type. Null means the type has no namespace. The
    // borrowed pointer stays valid while `ty` is held.
    let namespace = unsafe {
        let dict = (*ty.as_type_ptr()).tp_dict;
        if dict.is_null() {
            None
        } else {
            Some(Bound::from_borrowed_ptr(ty.py(), dict))
        }
    };
    for name in key.specials.iter() {
        if let Some(namespace) = &namespace
            && namespace.contains(name.as_ref())?
        {
            continue;
        }
        install_special(&ty, name)?;
    }
    install_sequence_length(&ty);
    Ok(ty)
}

thread_local! {
    static SEQUENCE_SLOT_INSTALLING: Cell<bool> = const { Cell::new(false) };
}

/// Clears [`SEQUENCE_SLOT_INSTALLING`] if slot installation re-enters.
struct SequenceSlotInstallGuard;

impl Drop for SequenceSlotInstallGuard {
    fn drop(&mut self) {
        SEQUENCE_SLOT_INSTALLING.with(|installing| installing.set(false));
    }
}

fn ensure_sequence_length(py: Python<'_>) {
    static ONCE: Once = Once::new();
    // `type_object` can run while this install is in progress. Waiting on the
    // same `Once` from that call would panic, and waiting while attached can
    // deadlock the thread that holds the `Once` and needs the GIL.
    if ONCE.is_completed() || SEQUENCE_SLOT_INSTALLING.with(Cell::get) {
        return;
    }
    ONCE.call_once_py_attached(py, || {
        SEQUENCE_SLOT_INSTALLING.with(|installing| installing.set(true));
        let _guard = SequenceSlotInstallGuard;
        install_sequence_length(&RustPythonObject::type_object(py));
        install_sequence_length(&RustPythonCallable::type_object(py));
        install_sequence_length(&RustPythonDataDescriptor::type_object(py));
    });
}

fn install_sequence_length(ty: &Bound<'_, PyType>) {
    // SAFETY: `ty` is a heap type. Its sequence slot struct is writable.
    // `PyMapping_Size` reads the mapping length and does not call this slot,
    // so the length function cannot recurse through itself.
    unsafe {
        let seq = (*ty.as_type_ptr()).tp_as_sequence;
        if seq.is_null() || (*seq).sq_length.is_some() {
            return;
        }
        (*seq).sq_length = Some(proxy_sequence_length);
    }
}

unsafe extern "C" fn proxy_sequence_length(obj: *mut ffi::PyObject) -> ffi::Py_ssize_t {
    unsafe { ffi::PyMapping_Size(obj) }
}

/// How many class name-sets to remember. Dead classes are not retained; an
/// entry is only an id, versions, and the names.
const SPECIALS_CACHE_CAP: usize = 1024;

type RpClass = rustpython_vm::Py<rustpython_vm::builtins::PyType>;

struct CachedSpecials {
    versions: Arc<[(usize, u32)]>,
    specials: Arc<[Box<str>]>,
}

static CLASS_SPECIALS: LazyLock<Mutex<HashMap<usize, CachedSpecials>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn cached_published_specials(
    py: Python<'_>,
    obj: &PyObject,
    vm: &VirtualMachine,
) -> Arc<[Box<str>]> {
    let class = obj.class();
    let id = class.get_id();
    if let Some(specials) = lookup_cached_specials(py, class, id, vm) {
        return specials;
    }
    if !ensure_mro_versions(class, vm) {
        return published_specials(class, vm);
    }
    let Some(versions) = mro_versions(class, vm) else {
        return published_specials(class, vm);
    };
    let specials = published_specials(class, vm);
    let Some(after) = mro_versions(class, vm) else {
        return specials;
    };
    if versions != after {
        return specials;
    }
    store_cached_specials(py, id, versions, Arc::clone(&specials));
    specials
}

fn lookup_cached_specials(
    py: Python<'_>,
    class: &RpClass,
    id: usize,
    vm: &VirtualMachine,
) -> Option<Arc<[Box<str>]>> {
    let found = {
        let cache = CLASS_SPECIALS
            .lock_py_attached(py)
            .unwrap_or_else(|err| err.into_inner());
        cache
            .get(&id)
            .map(|entry| (Arc::clone(&entry.versions), Arc::clone(&entry.specials)))
    };
    let (versions, specials) = found?;
    mro_versions_match(class, vm, &versions).then_some(specials)
}

fn store_cached_specials(
    py: Python<'_>,
    id: usize,
    versions: Arc<[(usize, u32)]>,
    specials: Arc<[Box<str>]>,
) {
    let mut cache = CLASS_SPECIALS
        .lock_py_attached(py)
        .unwrap_or_else(|err| err.into_inner());
    if !cache.contains_key(&id) && cache.len() >= SPECIALS_CACHE_CAP {
        let stale = cache.keys().next().copied();
        if let Some(stale) = stale
            && stale != id
        {
            cache.remove(&stale);
        }
    }
    cache.insert(id, CachedSpecials { versions, specials });
}

/// Return this class's version tag, assigning one when the tag is clear.
///
/// A modification clears the tag. Zero means both "never assigned" and
/// "already cleared", so nothing is cached under zero.
fn ensure_version(class: &RpClass, vm: &VirtualMachine) -> u32 {
    class.assign_version_tag(vm)
}

fn ensure_mro_versions(class: &RpClass, vm: &VirtualMachine) -> bool {
    let classes = class.mro_collect();
    for cls in &classes {
        if cls.is(vm.ctx.types.object_type) {
            continue;
        }
        if ensure_version(cls, vm) == 0 {
            return false;
        }
    }
    true
}

fn mro_versions(class: &RpClass, vm: &VirtualMachine) -> Option<Arc<[(usize, u32)]>> {
    let mro = class.mro.read();
    let mut versions = Vec::new();
    for cls in mro.iter() {
        if cls.is(vm.ctx.types.object_type) {
            continue;
        }
        let version = cls.tp_version_tag.load(Ordering::Acquire);
        if version == 0 {
            return None;
        }
        versions.push((cls.get_id(), version));
    }
    Some(Arc::from(versions))
}

fn mro_versions_match(class: &RpClass, vm: &VirtualMachine, versions: &[(usize, u32)]) -> bool {
    let mro = class.mro.read();
    let mut index = 0;
    for cls in mro.iter() {
        if cls.is(vm.ctx.types.object_type) {
            continue;
        }
        let Some(&(id, version)) = versions.get(index) else {
            return false;
        };
        if cls.get_id() != id || cls.tp_version_tag.load(Ordering::Acquire) != version {
            return false;
        }
        index += 1;
    }
    index == versions.len()
}

/// Dunder functions defined on `class`, excluding slots this proxy fills.
///
/// The first defining class in the MRO wins. `object` is not a defining
/// class: names only it has are not published, and a name this class defines
/// is published even when `object` also has it.
fn published_specials(class: &RpClass, vm: &VirtualMachine) -> Arc<[Box<str>]> {
    let mut decision = BTreeMap::new();
    for cls in class.mro_collect() {
        if cls.is(vm.ctx.types.object_type) {
            continue;
        }
        for (key, value) in cls.attributes.entries() {
            let Some(key) = key.downcast_ref::<rustpython_vm::builtins::PyStr>() else {
                continue;
            };
            let Some(text) = key.to_str() else {
                continue;
            };
            if !is_dunder(text) || is_proxy_slot(text) {
                continue;
            }
            decision
                .entry(text.to_owned())
                .or_insert_with(|| publishes_as_method(&value, vm));
        }
    }
    decision
        .into_iter()
        .filter_map(|(name, publish)| publish.then(|| name.into_boxed_str()))
        .collect()
}

fn is_dunder(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() > 4 && bytes.starts_with(b"__") && bytes.ends_with(b"__")
}

/// Slots the proxy class already fills, and names that belong to the proxy
/// type itself rather than to the wrapped instance.
fn is_proxy_slot(name: &str) -> bool {
    matches!(
        name,
        "__abs__"
            | "__add__"
            | "__aenter__"
            | "__aexit__"
            | "__aiter__"
            | "__and__"
            | "__anext__"
            | "__annotations__"
            | "__await__"
            | "__base__"
            | "__bases__"
            | "__bool__"
            | "__buffer__"
            | "__bytes__"
            | "__call__"
            | "__class__"
            | "__class_getitem__"
            | "__complex__"
            | "__contains__"
            | "__delattr__"
            | "__delete__"
            | "__delitem__"
            | "__dict__"
            | "__divmod__"
            | "__doc__"
            | "__enter__"
            | "__eq__"
            | "__exit__"
            | "__float__"
            | "__floordiv__"
            | "__ge__"
            | "__get__"
            | "__getattr__"
            | "__getattribute__"
            | "__getitem__"
            | "__gt__"
            | "__hash__"
            | "__iadd__"
            | "__iand__"
            | "__ifloordiv__"
            | "__ilshift__"
            | "__imatmul__"
            | "__imod__"
            | "__imul__"
            | "__index__"
            | "__init__"
            | "__init_subclass__"
            | "__instancecheck__"
            | "__int__"
            | "__invert__"
            | "__ior__"
            | "__ipow__"
            | "__irshift__"
            | "__isub__"
            | "__iter__"
            | "__itruediv__"
            | "__ixor__"
            | "__le__"
            | "__len__"
            | "__lshift__"
            | "__lt__"
            | "__matmul__"
            | "__mod__"
            | "__module__"
            | "__mro__"
            | "__mro_entries__"
            | "__mul__"
            | "__name__"
            | "__ne__"
            | "__neg__"
            | "__new__"
            | "__next__"
            | "__or__"
            | "__orig_bases__"
            | "__parameters__"
            | "__pos__"
            | "__pow__"
            | "__prepare__"
            | "__qualname__"
            | "__radd__"
            | "__rand__"
            | "__rdivmod__"
            | "__release_buffer__"
            | "__repr__"
            | "__rfloordiv__"
            | "__rlshift__"
            | "__rmatmul__"
            | "__rmod__"
            | "__rmul__"
            | "__ror__"
            | "__rpow__"
            | "__rrshift__"
            | "__rshift__"
            | "__rsub__"
            | "__rtruediv__"
            | "__rxor__"
            | "__set__"
            | "__setattr__"
            | "__setitem__"
            | "__slots__"
            | "__str__"
            | "__sub__"
            | "__subclasscheck__"
            | "__subclasshook__"
            | "__subclasses__"
            | "__text_signature__"
            | "__truediv__"
            | "__weakref__"
            | "__xor__"
    )
}

fn publishes_as_method(value: &PyObject, vm: &VirtualMachine) -> bool {
    if value.class().slots.descr_set.load().is_some() {
        return false;
    }
    let cls = value.class();
    let function =
        cls.is(vm.ctx.types.function_type) || cls.is(vm.ctx.types.builtin_function_or_method_type);
    if cls.slots.descr_get.load().is_some() {
        return function;
    }
    function || value.is_callable()
}

/// A plain function published on a proxy class.
///
/// The callable's `self` would be the closure data, so it cannot also bind
/// the instance. A descriptor does both: lookup on the class returns this
/// callable, and lookup on an instance returns one that already holds the
/// instance.
#[pyo3::pyclass(module = "rustpython", name = "published_method", frozen)]
struct PublishedMethod {
    name: String,
}

#[pyo3::pymethods]
impl PublishedMethod {
    fn __get__<'py>(
        slf: &Bound<'py, Self>,
        instance: Option<&Bound<'py, PyAny>>,
        _owner: Option<&Bound<'py, PyAny>>,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        match instance.filter(|instance| !instance.is_none()) {
            Some(instance) => {
                let bound = BoundPublished {
                    name: slf.borrow().name.clone(),
                    instance: instance.clone().unbind(),
                };
                Ok(Bound::new(slf.py(), bound)?.into_any())
            }
            None => Ok(slf.clone().into_any()),
        }
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn __call__<'py>(
        slf: &Bound<'py, Self>,
        args: &Bound<'py, PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        let name = slf.borrow().name.clone();
        call_published(&name, args, kwargs)
    }
}

/// [`PublishedMethod`] bound to one proxy instance.
#[pyo3::pyclass(module = "rustpython", name = "bound_published_method", frozen)]
struct BoundPublished {
    name: String,
    instance: Py<PyAny>,
}

#[pyo3::pymethods]
impl BoundPublished {
    #[pyo3(signature = (*args, **kwargs))]
    fn __call__<'py>(
        slf: &Bound<'py, Self>,
        args: &Bound<'py, PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let (name, instance) = {
            let this = slf.borrow();
            (this.name.clone(), this.instance.clone_ref(py))
        };
        let mut elements = Vec::with_capacity(args.len() + 1);
        elements.push(instance);
        elements.extend(args.iter().map(|item| item.clone().unbind()));
        let full = PyTuple::new(py, elements)?;
        call_published(&name, &full, kwargs)
    }
}

fn install_special(ty: &Bound<'_, PyType>, name: &str) -> pyo3::PyResult<()> {
    let func = Bound::new(
        ty.py(),
        PublishedMethod {
            name: name.to_owned(),
        },
    )?;
    ty.setattr(name, func)
}

fn call_published<'py>(
    name: &str,
    args: &Bound<'py, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> pyo3::PyResult<Bound<'py, PyAny>> {
    if PyTupleMethods::len(args) == 0 {
        return Err(PyTypeError::new_err(format!(
            "{name}() missing the wrapped object"
        )));
    }
    let instance = args.get_item(0)?;
    let proxy = instance.cast::<RustPythonObject>()?;
    let rest = PyTupleMethods::get_slice(args, 1, PyTupleMethods::len(args));
    proxy.borrow().run_for_cpython(
        args.py(),
        |vm| {
            let name = vm.ctx.new_str(name);
            Ok((name, args_to_rustpython(&rest, kwargs, vm)?))
        },
        |obj, (name, call_args), vm| obj.get_attr(&name, vm)?.call_with_args(call_args, vm),
    )
}

fn install_method(
    ty: &Bound<'_, PyType>,
    name: &str,
    def: &'static ffi::PyMethodDef,
) -> pyo3::PyResult<()> {
    let py = ty.py();
    // SAFETY: `ty` is a ready heap type. `def` points at a static `PyMethodDef`
    // that no one writes. The new descriptor keeps that pointer; the returned
    // pointer is a new reference, or null with an exception set.
    let descr =
        unsafe { ffi::PyDescr_NewMethod(ty.as_ptr().cast(), core::ptr::from_ref(def).cast_mut()) };
    if descr.is_null() {
        return Err(pyo3::PyErr::fetch(py));
    }
    // SAFETY: `descr` is a new reference and is not null.
    let descr = unsafe { Bound::from_owned_ptr(py, descr) };
    ty.setattr(name, descr)
}

/// [`ffi::PyMethodDef`] holds raw pointers, so it is not `Sync` on its own.
/// A static definition only points at a static name and a static function,
/// and nothing writes it.
struct SharedMethodDef(ffi::PyMethodDef);

// SAFETY: the bytes are immutable and contain only static pointers.
unsafe impl Sync for SharedMethodDef {}

const fn py_method(
    name: &'static core::ffi::CStr,
    meth: ffi::PyCFunction,
    flags: c_int,
) -> ffi::PyMethodDef {
    ffi::PyMethodDef {
        ml_name: name.as_ptr(),
        ml_meth: ffi::PyMethodDefPointer { PyCFunction: meth },
        ml_flags: flags,
        ml_doc: core::ptr::null(),
    }
}

const fn py_method_keywords(
    name: &'static core::ffi::CStr,
    meth: ffi::PyCFunctionWithKeywords,
) -> ffi::PyMethodDef {
    ffi::PyMethodDef {
        ml_name: name.as_ptr(),
        ml_meth: ffi::PyMethodDefPointer {
            PyCFunctionWithKeywords: meth,
        },
        ml_flags: ffi::METH_VARARGS | ffi::METH_KEYWORDS,
        ml_doc: core::ptr::null(),
    }
}

struct NumberMethod {
    bit: u64,
    name: &'static str,
    def: &'static SharedMethodDef,
    present: fn(&PyObject, &VirtualMachine) -> bool,
}

fn finish(py: Python<'_>, result: pyo3::PyResult<Bound<'_, PyAny>>) -> *mut ffi::PyObject {
    match result {
        Ok(value) => value.into_ptr(),
        Err(err) => {
            err.restore(py);
            core::ptr::null_mut()
        }
    }
}

fn borrowed_proxy(
    py: Python<'_>,
    slf: *mut ffi::PyObject,
) -> pyo3::PyResult<Bound<'_, RustPythonObject>> {
    // SAFETY: `slf` is the receiver of this call and stays alive for it.
    let obj = unsafe { Bound::from_borrowed_ptr(py, slf) };
    obj.cast::<RustPythonObject>().cloned().map_err(Into::into)
}

fn binary_call(
    py: Python<'_>,
    slf: *mut ffi::PyObject,
    other: *mut ffi::PyObject,
    op: PyNumberBinaryOp,
    reflected: bool,
) -> pyo3::PyResult<Bound<'_, PyAny>> {
    let proxy = borrowed_proxy(py, slf)?;
    // SAFETY: `other` is the single argument of this call.
    let other = unsafe { Bound::from_borrowed_ptr(py, other) };
    let proxy = proxy.borrow();
    proxy.forward_binary(&other, op, reflected)
}

fn unary_call(
    py: Python<'_>,
    slf: *mut ffi::PyObject,
    slot: fn(&PyNumberSlots) -> Option<PyNumberUnaryFunc>,
) -> pyo3::PyResult<Bound<'_, PyAny>> {
    let proxy = borrowed_proxy(py, slf)?;
    let proxy = proxy.borrow();
    proxy.forward_unary(py, slot)
}

fn direct_call<'py>(
    py: Python<'py>,
    slf: *mut ffi::PyObject,
    call: fn(&RustPythonObject, Python<'py>) -> pyo3::PyResult<Bound<'py, PyAny>>,
) -> pyo3::PyResult<Bound<'py, PyAny>> {
    let proxy = borrowed_proxy(py, slf)?;
    let proxy = proxy.borrow();
    call(&proxy, py)
}

fn power_call(
    py: Python<'_>,
    slf: *mut ffi::PyObject,
    args: *mut ffi::PyObject,
    reflected: bool,
) -> pyo3::PyResult<Bound<'_, PyAny>> {
    let proxy = borrowed_proxy(py, slf)?;
    // SAFETY: `args` is the argument tuple of this call.
    let args = unsafe { Bound::from_borrowed_ptr(py, args) };
    let args = args.cast::<PyTuple>()?;
    let proxy = proxy.borrow();
    proxy.forward_power(args, reflected)
}

fn none_result(py: Python<'_>, result: pyo3::PyResult<()>) -> pyo3::PyResult<Bound<'_, PyAny>> {
    result.map(|()| py.None().into_bound(py))
}

fn set_call(
    py: Python<'_>,
    slf: *mut ffi::PyObject,
    args: *mut ffi::PyObject,
) -> pyo3::PyResult<Bound<'_, PyAny>> {
    let proxy = borrowed_proxy(py, slf)?;
    // SAFETY: `args` is the argument tuple of this call.
    let args = unsafe { Bound::from_borrowed_ptr(py, args) };
    let args = args.cast::<PyTuple>()?;
    let instance = args.get_item(0)?;
    let value = args.get_item(1)?;
    none_result(py, descr_set(&proxy, &instance, Some(&value)))
}

fn delete_call(
    py: Python<'_>,
    slf: *mut ffi::PyObject,
    instance: *mut ffi::PyObject,
) -> pyo3::PyResult<Bound<'_, PyAny>> {
    let proxy = borrowed_proxy(py, slf)?;
    // SAFETY: `instance` is the single argument of this call.
    let instance = unsafe { Bound::from_borrowed_ptr(py, instance) };
    none_result(py, descr_set(&proxy, &instance, None))
}

fn call_from_c(
    py: Python<'_>,
    slf: *mut ffi::PyObject,
    args: *mut ffi::PyObject,
    kwargs: *mut ffi::PyObject,
) -> pyo3::PyResult<Bound<'_, PyAny>> {
    let proxy = borrowed_proxy(py, slf)?;
    // SAFETY: `args` is the positional tuple. `kwargs` is null or a dict.
    let args = unsafe { Bound::from_borrowed_ptr(py, args) };
    let args = args.cast::<PyTuple>()?;
    let kwargs = if kwargs.is_null() {
        None
    } else {
        let kwargs = unsafe { Bound::from_borrowed_ptr(py, kwargs) };
        Some(kwargs.cast::<PyDict>()?.clone())
    };
    call_object(&proxy, args, kwargs.as_ref())
}

// The interpreter holds the GIL for every trampoline below.
macro_rules! number_methods {
    ($([$kind:ident, $fn_name:ident, $def:ident, $has:ident, $cname:literal, $pyname:literal, $bit:literal $($rest:tt)*])*) => {
        $(number_methods!(@method $kind, $fn_name, $def, $has, $cname, $pyname, $bit $($rest)*);)*

        static NUMBER_METHODS: &[NumberMethod] = &[
            $(
                NumberMethod {
                    bit: 1u64 << $bit,
                    name: $pyname,
                    def: &$def,
                    present: $has,
                },
            )*
        ];
    };
    (@method binary, $fn_name:ident, $def:ident, $has:ident, $cname:literal, $pyname:literal, $bit:literal, $field:ident, $op:expr, $reflected:literal) => {
        unsafe extern "C" fn $fn_name(
            slf: *mut ffi::PyObject,
            other: *mut ffi::PyObject,
        ) -> *mut ffi::PyObject {
            unsafe {
                let py = Python::assume_attached();
                finish(py, binary_call(py, slf, other, $op, $reflected))
            }
        }

        static $def: SharedMethodDef = SharedMethodDef(py_method($cname, $fn_name, ffi::METH_O));

        fn $has(obj: &PyObject, _vm: &VirtualMachine) -> bool {
            obj.class().slots.as_number.$field.load().is_some()
        }
    };
    (@method unary, $fn_name:ident, $def:ident, $has:ident, $cname:literal, $pyname:literal, $bit:literal, $field:ident) => {
        unsafe extern "C" fn $fn_name(
            slf: *mut ffi::PyObject,
            _arg: *mut ffi::PyObject,
        ) -> *mut ffi::PyObject {
            unsafe {
                let py = Python::assume_attached();
                finish(py, unary_call(py, slf, |slots| slots.$field.load()))
            }
        }

        static $def: SharedMethodDef =
            SharedMethodDef(py_method($cname, $fn_name, ffi::METH_NOARGS));

        fn $has(obj: &PyObject, _vm: &VirtualMachine) -> bool {
            obj.class().slots.as_number.$field.load().is_some()
        }
    };
    (@method power, $fn_name:ident, $def:ident, $has:ident, $cname:literal, $pyname:literal, $bit:literal, $field:ident, $reflected:literal) => {
        unsafe extern "C" fn $fn_name(
            slf: *mut ffi::PyObject,
            args: *mut ffi::PyObject,
        ) -> *mut ffi::PyObject {
            unsafe {
                let py = Python::assume_attached();
                finish(py, power_call(py, slf, args, $reflected))
            }
        }

        static $def: SharedMethodDef =
            SharedMethodDef(py_method($cname, $fn_name, ffi::METH_VARARGS));

        fn $has(obj: &PyObject, _vm: &VirtualMachine) -> bool {
            obj.class().slots.as_number.$field.load().is_some()
        }
    };
    (@method slot, $fn_name:ident, $def:ident, $has:ident, $cname:literal, $pyname:literal, $bit:literal, $call:ident, $field:ident) => {
        unsafe extern "C" fn $fn_name(
            slf: *mut ffi::PyObject,
            _arg: *mut ffi::PyObject,
        ) -> *mut ffi::PyObject {
            unsafe {
                let py = Python::assume_attached();
                finish(py, direct_call(py, slf, RustPythonObject::$call))
            }
        }

        static $def: SharedMethodDef =
            SharedMethodDef(py_method($cname, $fn_name, ffi::METH_NOARGS));

        fn $has(obj: &PyObject, _vm: &VirtualMachine) -> bool {
            obj.class().slots.as_number.$field.load().is_some()
        }
    };
    (@method complex, $fn_name:ident, $def:ident, $has:ident, $cname:literal, $pyname:literal, $bit:literal) => {
        unsafe extern "C" fn $fn_name(
            slf: *mut ffi::PyObject,
            _arg: *mut ffi::PyObject,
        ) -> *mut ffi::PyObject {
            unsafe {
                let py = Python::assume_attached();
                finish(py, direct_call(py, slf, RustPythonObject::nb_complex))
            }
        }

        static $def: SharedMethodDef =
            SharedMethodDef(py_method($cname, $fn_name, ffi::METH_NOARGS));

        fn $has(obj: &PyObject, vm: &VirtualMachine) -> bool {
            obj.class()
                .get_attr(vm.ctx.intern_str("__complex__"))
                .is_some()
        }
    };
}

number_methods! {
    [binary, nb_add, ADD_DEF, has_add, c"__add__", "__add__", 0, add, PyNumberBinaryOp::Add, false]
    [binary, nb_radd, RADD_DEF, has_radd, c"__radd__", "__radd__", 1, right_add, PyNumberBinaryOp::Add, true]
    [binary, nb_sub, SUB_DEF, has_sub, c"__sub__", "__sub__", 2, subtract, PyNumberBinaryOp::Subtract, false]
    [binary, nb_rsub, RSUB_DEF, has_rsub, c"__rsub__", "__rsub__", 3, right_subtract, PyNumberBinaryOp::Subtract, true]
    [binary, nb_mul, MUL_DEF, has_mul, c"__mul__", "__mul__", 4, multiply, PyNumberBinaryOp::Multiply, false]
    [binary, nb_rmul, RMUL_DEF, has_rmul, c"__rmul__", "__rmul__", 5, right_multiply, PyNumberBinaryOp::Multiply, true]
    [binary, nb_truediv, TRUEDIV_DEF, has_truediv, c"__truediv__", "__truediv__", 6, true_divide, PyNumberBinaryOp::TrueDivide, false]
    [binary, nb_rtruediv, RTRUEDIV_DEF, has_rtruediv, c"__rtruediv__", "__rtruediv__", 7, right_true_divide, PyNumberBinaryOp::TrueDivide, true]
    [binary, nb_floordiv, FLOORDIV_DEF, has_floordiv, c"__floordiv__", "__floordiv__", 8, floor_divide, PyNumberBinaryOp::FloorDivide, false]
    [binary, nb_rfloordiv, RFLOORDIV_DEF, has_rfloordiv, c"__rfloordiv__", "__rfloordiv__", 9, right_floor_divide, PyNumberBinaryOp::FloorDivide, true]
    [binary, nb_mod, MOD_DEF, has_mod, c"__mod__", "__mod__", 10, remainder, PyNumberBinaryOp::Remainder, false]
    [binary, nb_rmod, RMOD_DEF, has_rmod, c"__rmod__", "__rmod__", 11, right_remainder, PyNumberBinaryOp::Remainder, true]
    [binary, nb_matmul, MATMUL_DEF, has_matmul, c"__matmul__", "__matmul__", 12, matrix_multiply, PyNumberBinaryOp::MatrixMultiply, false]
    [binary, nb_rmatmul, RMATMUL_DEF, has_rmatmul, c"__rmatmul__", "__rmatmul__", 13, right_matrix_multiply, PyNumberBinaryOp::MatrixMultiply, true]
    [binary, nb_and, AND_DEF, has_and, c"__and__", "__and__", 14, and, PyNumberBinaryOp::And, false]
    [binary, nb_rand, RAND_DEF, has_rand, c"__rand__", "__rand__", 15, right_and, PyNumberBinaryOp::And, true]
    [binary, nb_or, OR_DEF, has_or, c"__or__", "__or__", 16, or, PyNumberBinaryOp::Or, false]
    [binary, nb_ror, ROR_DEF, has_ror, c"__ror__", "__ror__", 17, right_or, PyNumberBinaryOp::Or, true]
    [binary, nb_xor, XOR_DEF, has_xor, c"__xor__", "__xor__", 18, xor, PyNumberBinaryOp::Xor, false]
    [binary, nb_rxor, RXOR_DEF, has_rxor, c"__rxor__", "__rxor__", 19, right_xor, PyNumberBinaryOp::Xor, true]
    [binary, nb_lshift, LSHIFT_DEF, has_lshift, c"__lshift__", "__lshift__", 20, lshift, PyNumberBinaryOp::Lshift, false]
    [binary, nb_rlshift, RLSHIFT_DEF, has_rlshift, c"__rlshift__", "__rlshift__", 21, right_lshift, PyNumberBinaryOp::Lshift, true]
    [binary, nb_rshift, RSHIFT_DEF, has_rshift, c"__rshift__", "__rshift__", 22, rshift, PyNumberBinaryOp::Rshift, false]
    [binary, nb_rrshift, RRSHIFT_DEF, has_rrshift, c"__rrshift__", "__rrshift__", 23, right_rshift, PyNumberBinaryOp::Rshift, true]
    [power, nb_pow, POW_DEF, has_pow, c"__pow__", "__pow__", 24, power, false]
    [power, nb_rpow, RPOW_DEF, has_rpow, c"__rpow__", "__rpow__", 25, right_power, true]
    [unary, nb_neg, NEG_DEF, has_neg, c"__neg__", "__neg__", 26, negative]
    [unary, nb_pos, POS_DEF, has_pos, c"__pos__", "__pos__", 27, positive]
    [unary, nb_invert, INVERT_DEF, has_invert, c"__invert__", "__invert__", 28, invert]
    [unary, nb_abs, ABS_DEF, has_abs, c"__abs__", "__abs__", 29, absolute]
    [slot, nb_index_slot, INDEX_DEF, has_index, c"__index__", "__index__", 30, nb_index, index]
    [slot, nb_float_slot, FLOAT_DEF, has_float, c"__float__", "__float__", 31, nb_float, float]
    [complex, nb_complex_slot, COMPLEX_DEF, has_complex, c"__complex__", "__complex__", 32]
}

unsafe extern "C" fn proxy_call(
    slf: *mut ffi::PyObject,
    args: *mut ffi::PyObject,
    kwargs: *mut ffi::PyObject,
) -> *mut ffi::PyObject {
    unsafe {
        let py = Python::assume_attached();
        finish(py, call_from_c(py, slf, args, kwargs))
    }
}

unsafe extern "C" fn proxy_set(
    slf: *mut ffi::PyObject,
    args: *mut ffi::PyObject,
) -> *mut ffi::PyObject {
    unsafe {
        let py = Python::assume_attached();
        finish(py, set_call(py, slf, args))
    }
}

unsafe extern "C" fn proxy_delete(
    slf: *mut ffi::PyObject,
    instance: *mut ffi::PyObject,
) -> *mut ffi::PyObject {
    unsafe {
        let py = Python::assume_attached();
        finish(py, delete_call(py, slf, instance))
    }
}

static CALL_DEF: SharedMethodDef = SharedMethodDef(py_method_keywords(c"__call__", proxy_call));
static SET_DEF: SharedMethodDef =
    SharedMethodDef(py_method(c"__set__", proxy_set, ffi::METH_VARARGS));
static DELETE_DEF: SharedMethodDef =
    SharedMethodDef(py_method(c"__delete__", proxy_delete, ffi::METH_O));
static ITER_DEF: SharedMethodDef =
    SharedMethodDef(py_method(c"__iter__", proxy_iter, ffi::METH_NOARGS));
static NEXT_DEF: SharedMethodDef =
    SharedMethodDef(py_method(c"__next__", proxy_next, ffi::METH_NOARGS));

unsafe extern "C" fn proxy_iter(
    slf: *mut ffi::PyObject,
    _arg: *mut ffi::PyObject,
) -> *mut ffi::PyObject {
    unsafe {
        let py = Python::assume_attached();
        finish(py, direct_call(py, slf, RustPythonObject::iter_proxy))
    }
}

unsafe extern "C" fn proxy_next(
    slf: *mut ffi::PyObject,
    _arg: *mut ffi::PyObject,
) -> *mut ffi::PyObject {
    unsafe {
        let py = Python::assume_attached();
        finish(py, direct_call(py, slf, RustPythonObject::next_proxy))
    }
}

/// One acquisition shared by every view of a [`RustPythonBuffer`].
struct Exported {
    buffer: PyBuffer,
    format: CString,
    shape: Vec<ffi::Py_ssize_t>,
    strides: Vec<ffi::Py_ssize_t>,
}

struct ExportSlot {
    exported: Option<Exported>,
    /// Successful `__getbuffer__` calls that have not been released yet.
    views: usize,
}

struct ViewParts {
    buf: *mut c_void,
    len: ffi::Py_ssize_t,
    readonly: c_int,
    itemsize: ffi::Py_ssize_t,
    format: *mut c_char,
    ndim: c_int,
    shape: *mut ffi::Py_ssize_t,
    strides: *mut ffi::Py_ssize_t,
}

impl Exported {
    fn new(buffer: PyBuffer) -> pyo3::PyResult<Self> {
        let format = CString::new(buffer.desc.format.as_bytes())
            .map_err(|_| PyBufferError::new_err("buffer format contains a NUL character"))?;
        let shape = buffer.desc.shape().to_vec();
        let strides = buffer.desc.strides().to_vec();
        Ok(Self {
            buffer,
            format,
            shape,
            strides,
        })
    }

    fn view_parts(&self, flags: c_int) -> pyo3::PyResult<ViewParts> {
        let desc = &self.buffer.desc;
        if flags & ffi::PyBUF_WRITABLE == ffi::PyBUF_WRITABLE && desc.readonly {
            return Err(PyBufferError::new_err("buffer is read-only"));
        }
        if !desc.is_contiguous() && flags & ffi::PyBUF_STRIDES != ffi::PyBUF_STRIDES {
            return Err(PyBufferError::new_err("buffer is not contiguous"));
        }
        if desc.has_suboffsets() {
            return Err(PyBufferError::new_err("buffer is not contiguous"));
        }
        let offset = desc.offset;
        // The borrow only pins the bytes for this copy. The acquisition keeps
        // them fixed for as long as a view stores the pointer.
        let buf = {
            let bytes = self.buffer.obj_bytes();
            bytes.as_ptr().wrapping_offset(offset) as *mut c_void
        };
        let format = if flags & ffi::PyBUF_FORMAT == ffi::PyBUF_FORMAT {
            self.format.as_ptr().cast_mut()
        } else {
            core::ptr::null_mut()
        };
        let shape = if flags & ffi::PyBUF_ND == ffi::PyBUF_ND {
            self.shape.as_ptr().cast_mut()
        } else {
            core::ptr::null_mut()
        };
        let strides = if flags & ffi::PyBUF_STRIDES == ffi::PyBUF_STRIDES {
            self.strides.as_ptr().cast_mut()
        } else {
            core::ptr::null_mut()
        };
        Ok(ViewParts {
            buf,
            len: desc.len as ffi::Py_ssize_t,
            readonly: c_int::from(desc.readonly),
            itemsize: desc.itemsize as ffi::Py_ssize_t,
            format,
            ndim: self.shape.len() as c_int,
            shape,
            strides,
        })
    }
}

/// Exports the memory of a RustPython buffer to CPython.
///
/// One acquisition serves every view. It ends in `__releasebuffer__` when the
/// last view is released, which is what lets the exporter resize again.
/// Pointers in a live view address memory that acquisition keeps fixed, and
/// the view keeps this object alive through `obj`.
#[cfg_attr(
    feature = "threading",
    pyo3::pyclass(module = "rustpython", name = "buffer", frozen)
)]
#[cfg_attr(
    not(feature = "threading"),
    pyo3::pyclass(module = "rustpython", name = "buffer", frozen, unsendable)
)]
pub(crate) struct RustPythonBuffer {
    slot: DropAttached<Mutex<ExportSlot>>,
    owner: DropAttached<PyObjectRef>,
}

impl RustPythonBuffer {
    /// A CPython `memoryview` of `buffer`, exported by `owner`.
    pub(crate) fn memoryview(
        py: Python<'_>,
        owner: PyObjectRef,
        buffer: PyBuffer,
    ) -> pyo3::PyResult<Bound<'_, PyAny>> {
        let exporter = Bound::new(
            py,
            Self {
                slot: DropAttached::new(Mutex::new(ExportSlot {
                    exported: Some(Exported::new(buffer)?),
                    views: 0,
                })),
                owner: DropAttached::new(owner),
            },
        )?;
        Ok(PyMemoryView::from(exporter.as_any())?.into_any())
    }

    /// The RustPython object a CPython `memoryview` borrows, when the chain of
    /// `obj` reaches an exporter created here.
    pub(crate) fn memoryview_owner(obj: &Bound<'_, PyAny>) -> Option<PyObjectRef> {
        let mut current = obj.clone();
        for _ in 0..4 {
            if let Ok(buffer) = current.cast::<Self>() {
                return Some(buffer.borrow().owner.deref().clone());
            }
            if current.cast::<PyMemoryView>().is_err() {
                return None;
            }
            current = current.getattr(pyo3::intern!(current.py(), "obj")).ok()?;
            if current.is_none() {
                return None;
            }
        }
        None
    }

    fn acquire(py: Python<'_>, owner: PyObjectRef, flags: c_int) -> pyo3::PyResult<Exported> {
        rustpython_vm::vm::thread::try_with_current_vm(|vm| {
            vm.attach_for_callback(|| {
                let flags = BufferFlags::from_bits_truncate(flags as u32);
                let buffer = PyBuffer::from_object(vm, &owner, flags)
                    .map_err(|exc| crate::errors::rustpython_exc_to_cpython(py, exc, vm))?;
                Exported::new(buffer)
            })
        })
        .unwrap_or_else(|| {
            Err(pyo3::exceptions::PyRuntimeError::new_err(
                "no RustPython interpreter is running on this thread",
            ))
        })
    }

    /// Install an acquisition when the previous one has been released.
    fn ensure_exported(slf: &Bound<'_, Self>, flags: c_int) -> pyo3::PyResult<()> {
        let present = slf
            .borrow()
            .slot
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .exported
            .is_some();
        if present {
            return Ok(());
        }
        let owner = slf.borrow().owner.deref().clone();
        let exported = Self::acquire(slf.py(), owner, flags)?;
        let spare = {
            let this = slf.borrow();
            let mut slot = this.slot.lock().unwrap_or_else(|err| err.into_inner());
            if slot.exported.is_some() {
                Some(exported)
            } else {
                slot.exported = Some(exported);
                None
            }
        };
        if let Some(spare) = spare {
            drop(DropAttached::new(spare));
        }
        Ok(())
    }
}

#[pyo3::pymethods]
impl RustPythonBuffer {
    unsafe fn __getbuffer__(
        slf: Bound<'_, Self>,
        view: *mut ffi::Py_buffer,
        flags: c_int,
    ) -> pyo3::PyResult<()> {
        loop {
            Self::ensure_exported(&slf, flags)?;
            let this = slf.borrow();
            let mut slot = this.slot.lock().unwrap_or_else(|err| err.into_inner());
            let parts = {
                let Some(exported) = slot.exported.as_ref() else {
                    continue;
                };
                exported.view_parts(flags)?
            };
            slot.views += 1;
            let obj = slf.clone().into_any().into_ptr();
            // SAFETY: `view` is the caller's Py_buffer. The pointers address the
            // acquisition inside `slot`, which stays put until the matching
            // release takes it out. `obj` keeps this object alive that long.
            unsafe {
                (*view).obj = obj;
                (*view).buf = parts.buf;
                (*view).len = parts.len;
                (*view).readonly = parts.readonly;
                (*view).itemsize = parts.itemsize;
                (*view).format = parts.format;
                (*view).ndim = parts.ndim;
                (*view).shape = parts.shape;
                (*view).strides = parts.strides;
                (*view).suboffsets = core::ptr::null_mut();
                (*view).internal = core::ptr::null_mut();
            }
            return Ok(());
        }
    }

    unsafe fn __releasebuffer__(&self, _view: *mut ffi::Py_buffer) {
        let exported = {
            let mut slot = self.slot.lock().unwrap_or_else(|err| err.into_inner());
            if slot.views == 0 {
                return;
            }
            slot.views -= 1;
            if slot.views == 0 {
                slot.exported.take()
            } else {
                None
            }
        };
        // Releasing runs arbitrary Python, so the slot lock is already gone.
        if let Some(exported) = exported {
            drop(DropAttached::new(exported));
        }
    }
}
