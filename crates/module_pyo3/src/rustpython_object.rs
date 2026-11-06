//! RustPython objects as seen from CPython.
//!
//! [`RustPythonObject`] is the mirror image of `pyo3.ref`: a CPython object
//! that forwards attribute access, calls, item access, iteration, comparison
//! and the descriptor protocol to the RustPython object it wraps. A
//! RustPython function stored in a CPython class therefore binds to instances
//! like any other function, and a `property` keeps working as a property.
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
//! go. An object with no number protocol still uses the callable class, the
//! data descriptor class, or this base class. An object that has a number
//! protocol keeps `__call__` or `__set__` on that same proxy class when it is
//! callable or a data descriptor.

use alloc::ffi::CString;
use core::cell::RefCell;
use core::ffi::{c_char, c_int, c_void};
use core::ops::Deref;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use pyo3::exceptions::{PyBufferError, PyTypeError};
use pyo3::ffi;
use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use pyo3::type_object::PyTypeInfo;
use pyo3::types::{PyDict, PyMemoryView, PyString, PyTuple, PyType};
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
    bits
}

/// Whether `obj`'s class defines a number protocol this proxy can forward.
pub(crate) fn has_number_protocol(obj: &PyObject, vm: &VirtualMachine) -> bool {
    (proxy_bits(obj, vm) & !(CALL | DESCR)) != 0
}

impl RustPythonObject {
    /// Wrap `obj` in the proxy class matching what it supports.
    pub(crate) fn wrap<'py>(
        py: Python<'py>,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        let bits = proxy_bits(&obj, vm);
        let base = Self {
            obj: DropAttached::new(obj),
        };
        if (bits & !(CALL | DESCR)) == 0 {
            if (bits & CALL) != 0 {
                Bound::new(py, (RustPythonCallable, base)).map(Bound::into_any)
            } else if (bits & DESCR) != 0 {
                Bound::new(py, (RustPythonDataDescriptor, base)).map(Bound::into_any)
            } else {
                Bound::new(py, base).map(Bound::into_any)
            }
        } else {
            let ty = proxy_type(py, bits)?;
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

static PROXY_TYPES: LazyLock<Mutex<HashMap<u64, Py<PyType>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn proxy_type(py: Python<'_>, bits: u64) -> pyo3::PyResult<Bound<'_, PyType>> {
    if let Some(ty) = lookup_proxy_type(py, bits) {
        return Ok(ty);
    }
    let created = create_proxy_type(py, bits)?;
    // Hold the lock only while updating the map. Creating the type runs
    // Python and can re-enter this function.
    let stored = {
        let mut types = PROXY_TYPES.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(existing) = types.get(&bits) {
            existing.clone_ref(py)
        } else {
            let stored = created.unbind();
            types.insert(bits, stored.clone_ref(py));
            stored
        }
    };
    Ok(stored.into_bound(py))
}

fn lookup_proxy_type(py: Python<'_>, bits: u64) -> Option<Bound<'_, PyType>> {
    let ty = {
        let types = PROXY_TYPES.lock().unwrap_or_else(|err| err.into_inner());
        types.get(&bits).map(|ty| ty.clone_ref(py))
    };
    ty.map(|ty| ty.into_bound(py))
}

fn create_proxy_type(py: Python<'_>, bits: u64) -> pyo3::PyResult<Bound<'_, PyType>> {
    let type_ctor = py.import("builtins")?.getattr("type")?;
    let name = format!("number_{bits:x}");
    let base = RustPythonObject::type_object(py);
    let ty = type_ctor.call1((name, (base,), PyDict::new(py)))?;
    let ty = ty.cast::<PyType>()?.clone();
    ty.setattr("__module__", "rustpython")?;
    for method in NUMBER_METHODS {
        if (bits & method.bit) != 0 {
            install_method(&ty, method.name, &method.def.0)?;
        }
    }
    if (bits & CALL) != 0 {
        install_method(&ty, "__call__", &CALL_DEF.0)?;
    }
    if (bits & DESCR) != 0 {
        install_method(&ty, "__set__", &SET_DEF.0)?;
        install_method(&ty, "__delete__", &DELETE_DEF.0)?;
    }
    Ok(ty)
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
