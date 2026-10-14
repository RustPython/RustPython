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
//! - an exact `list`, `dict`, `set`, `frozenset` or `bytearray`, and the exact
//!   immutable built-ins above, are copied as that built-in. Code checks these
//!   types with `PyList_Check` and the like, which a proxy would fail; in
//!   exchange, changes to a copied `list`, `dict`, `set` or `bytearray` never
//!   reach the RustPython original;
//! - a subclass of those containers is pickled, so it arrives as the class the
//!   other interpreter imports under the same name. When pickling or
//!   unpickling fails, the subclass is copied as the base built-in from its
//!   stored contents and is never proxied. That copy has none of the
//!   subclass's methods, an exact-type check sees the base built-in, and
//!   writes do not reach the original;
//! - a subclass of `int`, `float`, `complex`, `str` or `bytes` is copied as a
//!   value of that built-in, from the stored contents rather than `__str__`
//!   or `__int__`. C code accepts the concrete type before any protocol, and
//!   no layout is both that built-in and a proxy;
//! - other buffers are exposed through a `memoryview` sharing their memory,
//!   including a buffer whose class also defines `read` or `write`. An
//!   indirect layout is not described from here;
//! - anything else that is callable, a data descriptor, or supports a number
//!   protocol becomes a `rustpython.object` proxy. With no number protocol and
//!   no iteration slots, a callable uses the callable class and a data
//!   descriptor the descriptor class. With a number protocol, or when the
//!   class defines iteration, the proxy class carries exactly the slots the
//!   wrapped class defines — `__index__`, `__float__`, `__complex__`, unary
//!   `- + ~ abs`, `+ - * / // % ** @ & | ^ << >>` with their reflected forms,
//!   and `__iter__` or `__next__` when those slots exist — and nothing it
//!   lacks, plus `__call__` or `__set__` when the object is also callable or
//!   a data descriptor. One class is cached per set of those slots;
//! - anything else is pickled when a copy cannot be told apart from the
//!   original by equality, and proxied otherwise. Equality is the class's
//!   `__eq__` found on the class MRO. When that object is `object.__eq__`,
//!   comparison is by identity, so a pickle copy would not compare equal to
//!   the original and the object is proxied, never pickled. Only a class that
//!   overrides `__eq__` is pickled. The lookup does not ask the instance, so
//!   a name only `__getattr__` would invent does not count.
//!
//! Keyword names of a call into CPython are interned. A callee that matches
//! keywords by identity then sees the names it was defined with.
//!
//! A `str` or `bytes` converted while calling into CPython is copied once
//! for that call: the same RustPython object passed twice arrives as one
//! object. The callee may keep a raw pointer into that copy and not own
//! the copy. The copies then live as long as that call's result: a weak
//! reference drops them once the result is gone, and only after its
//! destructor has returned. A result that cannot be weak-referenced leaves
//! the copies with the proxy of the result, or drops them when a non-proxy
//! result is converted back. Nothing keeps a copy after the result that
//! needed it is gone.

use core::cell::RefCell;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use malachite_bigint::BigInt;
use num_traits::ToPrimitive;
use pyo3::prelude::*;
use pyo3::types::{
    PyBool, PyByteArray, PyBytes, PyCFunction, PyComplex, PyDict, PyFloat, PyFrozenSet, PyInt,
    PyList, PyMemoryView, PySet, PyString, PyTuple, PyType as CPythonType, PyWeakrefReference,
};
use rustpython_vm::{
    AsObject, Py, PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, VirtualMachine,
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
use crate::rustpython_object::{
    RustPythonBuffer, RustPythonObject, has_number_protocol, unwrap_rustpython_object,
};

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
        if let Some(value) = value_to_cpython(py, obj, vm)? {
            return Ok(Some(value));
        }
        container_to_cpython(py, obj, vm)
    })? {
        return Ok(value);
    }
    // A strided view is shared, even when the class also has `read` or
    // `write`. An indirect layout is not described from here.
    if let Ok(buffer) = PyBuffer::try_from_borrowed_object(vm, obj)
        && !buffer.desc.has_suboffsets()
    {
        return RustPythonBuffer::memoryview(py, obj.to_owned(), buffer).to_pyresult(vm);
    }
    if obj.is_callable()
        || obj.class().slots.descr_get.load().is_some()
        || has_number_protocol(obj, vm)
    {
        return RustPythonObject::wrap(py, obj.to_owned(), vm).to_pyresult(vm);
    }
    // A copy is acceptable only when equality cannot tell it from the original.
    if compares_by_value(obj, vm)
        && let Some(pickled) = pickle_to_cpython(py, obj, vm)
    {
        return Ok(pickled);
    }
    RustPythonObject::wrap(py, obj.to_owned(), vm).to_pyresult(vm)
}

/// `true` when the class MRO's `__eq__` is not `object.__eq__`.
///
/// `object.__eq__` compares by identity. A class that does not override it
/// would see a pickled copy as a different object, so that copy is refused.
/// The name is taken from the class, not from the instance.
fn compares_by_value(obj: &PyObject, vm: &VirtualMachine) -> bool {
    let name = vm.ctx.intern_str("__eq__");
    let Some(class_eq) = obj.class().get_attr(name) else {
        return false;
    };
    let Some(object_eq) = vm.ctx.types.object_type.get_attr(name) else {
        return false;
    };
    !class_eq.is(&object_eq)
}

fn to_rustpython_inner(obj: &Bound<'_, PyAny>, vm: &VirtualMachine) -> PyResult {
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
    // proxy payload. When the exporter is ours, `.obj` is the RustPython
    // object whose memory was shared, not the exporter itself.
    if obj.cast::<PyMemoryView>().is_ok() {
        if let Some(owner) = RustPythonBuffer::memoryview_owner(obj)
            && let Ok(buffer) = PyBuffer::try_from_borrowed_object(vm, &owner)
        {
            return Ok(rustpython_vm::PyPayload::into_pyobject(
                rustpython_vm::builtins::PyMemoryView::from_buffer(buffer, vm)?,
                vm,
            ));
        }
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
    // One RustPython object converted twice in this call is one object on
    // the far side. The copies stay alive as long as `result` does.
    with_value_memo(|| {
        let positional = items_to_cpython(py, &args.args, vm)?;
        let result = call_cpython_with(callable, positional, &args.kwargs, vm)?;
        retain_call_copies(&result, vm)?;
        Ok(result)
    })
}

/// Convert a CPython object for use in RustPython.
pub(crate) fn to_rustpython(obj: &Bound<'_, PyAny>, vm: &VirtualMachine) -> PyResult {
    // Copies stashed for `obj` are claimed by its proxy. Anything left over
    // belongs to a result that is not a proxy and is released with this call.
    let _release = ReleaseUnclaimed(obj.as_ptr() as usize);
    to_rustpython_inner(obj, vm)
}

struct ReleaseUnclaimed(usize);
impl Drop for ReleaseUnclaimed {
    fn drop(&mut self) {
        drop(take_pending_copies(self.0));
    }
}

std::thread_local! {
    static VALUE_MEMO: RefCell<Vec<HashMap<usize, pyo3::Py<PyAny>>>> =
        const { RefCell::new(Vec::new()) };
    /// Copies from a call, keyed by the result object, until its proxy claims them.
    static PENDING_COPIES: RefCell<HashMap<usize, Vec<pyo3::Py<PyAny>>>> =
        RefCell::new(HashMap::new());
}

/// Share immutable values for the duration of one call.
fn with_value_memo<R>(f: impl FnOnce() -> R) -> R {
    VALUE_MEMO.with(|slot| slot.borrow_mut().push(HashMap::new()));
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            VALUE_MEMO.with(|slot| {
                slot.borrow_mut().pop();
            });
        }
    }
    let _guard = Guard;
    f()
}

fn cached_value<'py>(py: Python<'py>, obj: &PyObject) -> Option<Bound<'py, PyAny>> {
    VALUE_MEMO.with(|slot| {
        let stack = slot.borrow();
        let cached = stack.last()?.get(&(obj as *const PyObject as usize))?;
        Some(cached.bind(py).clone())
    })
}

fn remember_value(obj: &PyObject, value: &Bound<'_, PyAny>) {
    VALUE_MEMO.with(|slot| {
        let mut stack = slot.borrow_mut();
        let Some(map) = stack.last_mut() else {
            return;
        };
        map.insert(obj as *const PyObject as usize, value.clone().unbind());
    });
}

/// Copies kept alive for one call result, plus the weak reference that
/// drops them when that result is released.
struct AnchoredCopies {
    copies: Vec<pyo3::Py<PyAny>>,
    weakref: Option<pyo3::Py<PyAny>>,
}

impl Drop for AnchoredCopies {
    fn drop(&mut self) {
        // The copies first: the weak reference's callback has already run,
        // and dropping it must not be what releases the bytes it guarded.
        let copies = core::mem::take(&mut self.copies);
        let weakref = self.weakref.take();
        drop(copies);
        drop(weakref);
    }
}

fn anchored_copies() -> std::sync::MutexGuard<'static, HashMap<usize, AnchoredCopies>> {
    static ANCHORED: LazyLock<Mutex<HashMap<usize, AnchoredCopies>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    ANCHORED.lock().unwrap_or_else(|err| err.into_inner())
}

std::thread_local! {
    /// Copies whose result has started destruction. Dropped only from
    /// [`flush_deferred_drops`], after that destructor has returned.
    static DEFERRED_DROPS: RefCell<Vec<AnchoredCopies>> = const { RefCell::new(Vec::new()) };
}

fn defer_anchored(anchored: AnchoredCopies) {
    DEFERRED_DROPS.with(|slot| slot.borrow_mut().push(anchored));
}

/// Drop copies queued by a weak-reference callback.
///
/// The callback runs at the start of the result's destructor, so the copies
/// have to outlive the rest of that destructor. Called with the GIL held.
pub(crate) fn flush_deferred_drops() {
    loop {
        let batch = DEFERRED_DROPS.with(|slot| {
            let mut queue = slot.borrow_mut();
            if queue.is_empty() {
                return Vec::new();
            }
            core::mem::take(&mut *queue)
        });
        if batch.is_empty() {
            break;
        }
        drop(batch);
    }
}

fn release_anchored(id: usize) {
    let anchored = {
        let mut map = anchored_copies();
        map.remove(&id)
    };
    if let Some(anchored) = anchored {
        defer_anchored(anchored);
    }
}

enum Anchor {
    Kept,
    /// `result` cannot be weak-referenced. The caller keeps `copies`.
    Unsupported(Vec<pyo3::Py<PyAny>>),
    Error(pyo3::PyErr),
}

/// Keep `copies` alive until `result` is released.
///
/// The weak reference itself is held here: the referent does not own it, so
/// dropping it would unregister the callback. `copies` does not include
/// `result`, or the result could never be released.
fn anchor_to_result(result: &Bound<'_, PyAny>, copies: Vec<pyo3::Py<PyAny>>) -> Anchor {
    static NEXT_ANCHOR: AtomicUsize = AtomicUsize::new(1);
    let id = NEXT_ANCHOR.fetch_add(1, Ordering::Relaxed);
    let py = result.py();
    let callback = match PyCFunction::new_closure(
        py,
        None,
        None,
        move |_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
            release_anchored(id);
            Ok::<(), pyo3::PyErr>(())
        },
    ) {
        Ok(callback) => callback,
        Err(err) => return Anchor::Error(err),
    };
    match PyWeakrefReference::new_with(result, &callback) {
        Ok(weakref) => {
            anchored_copies().insert(
                id,
                AnchoredCopies {
                    copies,
                    weakref: Some(weakref.into_any().unbind()),
                },
            );
            Anchor::Kept
        }
        Err(err) if err.is_instance_of::<pyo3::exceptions::PyTypeError>(py) => {
            Anchor::Unsupported(copies)
        }
        Err(err) => Anchor::Error(err),
    }
}

/// Keep this call's copies alive as long as `result` may hold a pointer into them.
fn retain_call_copies(result: &Bound<'_, PyAny>, vm: &VirtualMachine) -> PyResult<()> {
    let mut copies = VALUE_MEMO.with(|slot| {
        let mut stack = slot.borrow_mut();
        let Some(map) = stack.last_mut() else {
            return Vec::new();
        };
        map.drain().map(|(_, value)| value).collect::<Vec<_>>()
    });
    let result_ptr = result.as_ptr();
    copies.retain(|copy| copy.as_ptr() != result_ptr);
    if copies.is_empty() {
        return Ok(());
    }
    let copies = match anchor_to_result(result, copies) {
        Anchor::Kept => return Ok(()),
        Anchor::Unsupported(copies) => copies,
        Anchor::Error(err) => return Err(err).to_pyresult(vm),
    };
    let key = result_ptr as usize;
    if key == 0 {
        return Ok(());
    }
    PENDING_COPIES.with(|slot| {
        slot.borrow_mut().entry(key).or_default().extend(copies);
    });
    Ok(())
}

/// Copies a call left for the object at `key`, if its proxy has not taken them.
pub(crate) fn take_pending_copies(key: usize) -> Vec<pyo3::Py<PyAny>> {
    if key == 0 {
        return Vec::new();
    }
    PENDING_COPIES.with(|slot| slot.borrow_mut().remove(&key).unwrap_or_default())
}

fn memoized_immutable<'py>(
    py: Python<'py>,
    obj: &PyObject,
    make: impl FnOnce() -> PyResult<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    if let Some(cached) = cached_value(py, obj) {
        return Ok(cached);
    }
    let value = make()?;
    remember_value(obj, &value);
    Ok(value)
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
        // A definition's keyword names are interned. Match them by identity.
        let key = match name.as_str() {
            Ok(text) => PyString::intern(py, text).into_any(),
            Err(_) => str_to_cpython(py, name).to_pyresult(vm)?.into_any(),
        };
        keywords
            .set_item(key, to_cpython(py, value, vm)?)
            .to_pyresult(vm)?;
    }
    run_cpython(vm, || callable.call(positional, Some(&keywords)))
}

/// Copy an exact immutable built-in value.
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
        // `bool` shares the int payload, so it has to be matched before the
        // int branch. A bool subclass falls through and is copied as `int`.
        PyBool::new(py, obj.is(&ctx.true_value))
            .to_owned()
            .into_any()
    } else if let Some(int) = obj.downcast_ref::<RustPyInt>() {
        int_to_cpython(py, int.as_bigint()).to_pyresult(vm)?
    } else if let Some(float) = obj.downcast_ref::<RustPyFloat>() {
        PyFloat::new(py, float.to_f64()).into_any()
    } else if let Some(complex) = obj.downcast_ref::<RustPyComplex>() {
        let value = complex.to_complex();
        PyComplex::from_doubles(py, value.re, value.im).into_any()
    } else if let Some(s) = obj.downcast_ref::<PyStr>() {
        memoized_immutable(py, obj, || {
            Ok(str_to_cpython(py, s.as_wtf8()).to_pyresult(vm)?.into_any())
        })?
    } else if let Some(bytes) = obj.downcast_ref::<RustPyBytes>() {
        memoized_immutable(
            py,
            obj,
            || Ok(PyBytes::new(py, bytes.as_bytes()).into_any()),
        )?
    } else if let Some(tuple) = obj.downcast_ref_if_exact::<RustPyTuple>(vm) {
        let items = items_to_cpython(py, tuple.as_slice(), vm)?;
        PyTuple::new(py, items).to_pyresult(vm)?.into_any()
    } else {
        return Ok(None);
    };
    Ok(Some(value))
}

/// Which stored container `obj` carries, exact class or subclass.
#[derive(Clone, Copy)]
enum Container<'a> {
    ByteArray(&'a RustPyByteArray),
    List(&'a RustPyList),
    Set(&'a RustPySet),
    FrozenSet(&'a RustPyFrozenSet),
    Dict(&'a RustPyDict),
}

fn container_kind(obj: &PyObject) -> Option<Container<'_>> {
    if let Some(bytearray) = obj.downcast_ref::<RustPyByteArray>() {
        Some(Container::ByteArray(bytearray))
    } else if let Some(list) = obj.downcast_ref::<RustPyList>() {
        Some(Container::List(list))
    } else if let Some(set) = obj.downcast_ref::<RustPySet>() {
        Some(Container::Set(set))
    } else if let Some(set) = obj.downcast_ref::<RustPyFrozenSet>() {
        Some(Container::FrozenSet(set))
    } else {
        obj.downcast_ref::<RustPyDict>()
            .map(|dict| Container::Dict(dict))
    }
}

fn container_base_type(kind: Container<'_>, vm: &VirtualMachine) -> &'static Py<PyType> {
    match kind {
        Container::ByteArray(_) => vm.ctx.types.bytearray_type,
        Container::List(_) => vm.ctx.types.list_type,
        Container::Set(_) => vm.ctx.types.set_type,
        Container::FrozenSet(_) => vm.ctx.types.frozenset_type,
        Container::Dict(_) => vm.ctx.types.dict_type,
    }
}

/// An exact container is copied. A subclass is pickled so its class is kept
/// when the other interpreter can import it, and copied from its stored
/// contents when pickling or unpickling fails. A container is never proxied.
fn container_to_cpython<'py>(
    py: Python<'py>,
    obj: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let Some(container) = container_kind(obj) else {
        return Ok(None);
    };
    let pickled = if obj.class().is(container_base_type(container, vm)) {
        None
    } else {
        pickle_to_cpython(py, obj, vm)
    };
    let value = match pickled {
        Some(pickled) => pickled,
        None => copy_stored_container(py, container, vm)?,
    };
    Ok(Some(value))
}

/// Copy `list`, `dict`, `set`, `frozenset` or `bytearray` from the payload,
/// ignoring methods defined on a subclass.
fn copy_stored_container<'py>(
    py: Python<'py>,
    container: Container<'_>,
    vm: &VirtualMachine,
) -> PyResult<Bound<'py, PyAny>> {
    match container {
        Container::ByteArray(bytearray) => {
            Ok(PyByteArray::new(py, &bytearray.borrow_buf()).into_any())
        }
        Container::List(list) => {
            let items = list.borrow_vec().to_vec();
            let items = items_to_cpython(py, &items, vm)?;
            Ok(PyList::new(py, items).to_pyresult(vm)?.into_any())
        }
        Container::Set(set) => {
            let items = items_to_cpython(py, &set.elements(), vm)?;
            Ok(PySet::new(py, items).to_pyresult(vm)?.into_any())
        }
        Container::FrozenSet(set) => {
            let items = items_to_cpython(py, &set.elements(), vm)?;
            Ok(PyFrozenSet::new(py, items).to_pyresult(vm)?.into_any())
        }
        Container::Dict(dict) => {
            let converted = PyDict::new(py);
            for (key, value) in dict.items_vec() {
                converted
                    .set_item(to_cpython(py, &key, vm)?, to_cpython(py, &value, vm)?)
                    .to_pyresult(vm)?;
            }
            Ok(converted.into_any())
        }
    }
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
