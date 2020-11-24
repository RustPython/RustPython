/*
 * Builtin set type with a sequence of unique items.
 */
use super::pytype::PyTypeRef;
use crate::common::hash::PyHash;
use crate::common::rc::PyRc;
use crate::dictdatatype;
use crate::function::OptionalArg::{Missing, Present};
use crate::function::{Args, OptionalArg};
use crate::pyobject::{
    self, BorrowValue, IdProtocol, PyClassImpl, PyComparisonValue, PyContext, PyIterable,
    PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::slots::{Comparable, Hashable, Iterable, PyComparisonOp, PyIter, Unhashable};
use crate::vm::{ReprGuard, VirtualMachine};
use crossbeam_utils::atomic::AtomicCell;
use std::fmt;

pub type SetContentType = dictdatatype::Dict<()>;

/// set() -> new empty set object
/// set(iterable) -> new set object
///
/// Build an unordered collection of unique elements.
#[pyclass(module = false, name = "set")]
#[derive(Default)]
pub struct PySet {
    inner: PySetInner,
}
pub type PySetRef = PyRef<PySet>;

/// frozenset() -> empty frozenset object
/// frozenset(iterable) -> frozenset object
///
/// Build an immutable unordered collection of unique elements.
#[pyclass(module = false, name = "frozenset")]
#[derive(Default)]
pub struct PyFrozenSet {
    inner: PySetInner,
}

impl fmt::Debug for PySet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("set")
    }
}

impl fmt::Debug for PyFrozenSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("frozenset")
    }
}

impl PyValue for PySet {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.set_type
    }
}

impl PyValue for PyFrozenSet {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.frozenset_type
    }
}

#[derive(Default, Clone)]
struct PySetInner {
    content: PyRc<SetContentType>,
}

impl PySetInner {
    fn new(iterable: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let set = PySetInner::default();
        for item in iterable.iter(vm)? {
            set.add(item?, vm)?;
        }
        Ok(set)
    }

    fn from_arg(iterable: OptionalArg<PyIterable>, vm: &VirtualMachine) -> PyResult<PySetInner> {
        if let OptionalArg::Present(iterable) = iterable {
            Self::new(iterable, vm)
        } else {
            Ok(PySetInner::default())
        }
    }

    fn len(&self) -> usize {
        self.content.len()
    }

    fn sizeof(&self) -> usize {
        self.content.sizeof()
    }

    fn copy(&self) -> PySetInner {
        PySetInner {
            content: PyRc::new((*self.content).clone()),
        }
    }

    fn contains(&self, needle: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.content.contains(vm, needle)
    }

    fn compare(
        &self,
        other: &PySetInner,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        if op == PyComparisonOp::Ne {
            return self.compare(other, PyComparisonOp::Eq, vm).map(|eq| !eq);
        }
        if !op.eval_ord(self.len().cmp(&other.len())) {
            return Ok(false);
        }
        let (superset, subset) = if matches!(op, PyComparisonOp::Lt | PyComparisonOp::Le) {
            (other, self)
        } else {
            (self, other)
        };
        for key in subset.content.keys() {
            if !superset.contains(&key, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn union(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let set = self.clone();
        for item in other.iter(vm)? {
            set.add(item?, vm)?;
        }

        Ok(set)
    }

    fn intersection(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let set = PySetInner::default();
        for item in other.iter(vm)? {
            let obj = item?;
            if self.contains(&obj, vm)? {
                set.add(obj, vm)?;
            }
        }
        Ok(set)
    }

    fn difference(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let set = self.copy();
        for item in other.iter(vm)? {
            set.content.delete_if_exists(vm, &item?)?;
        }
        Ok(set)
    }

    fn symmetric_difference(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let new_inner = self.clone();

        // We want to remove duplicates in other
        let other_set = Self::new(other, vm)?;

        for item in other_set.content.keys() {
            new_inner.content.delete_or_insert(vm, &item, ())?
        }

        Ok(new_inner)
    }

    fn issuperset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        for item in other.iter(vm)? {
            if !self.contains(&item?, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn issubset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        let other_set = PySetInner::new(other, vm)?;
        self.compare(&other_set, PyComparisonOp::Le, vm)
    }

    fn isdisjoint(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        for item in other.iter(vm)? {
            if self.contains(&item?, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn iter(&self) -> PySetIterator {
        let set_size = SetSizeInfo {
            position: 0,
            size: Some(self.content.len()),
        };

        PySetIterator {
            dict: PyRc::clone(&self.content),
            size_info: AtomicCell::new(set_size),
        }
    }

    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        let mut str_parts = Vec::with_capacity(self.content.len());
        for key in self.content.keys() {
            let part = vm.to_repr(&key)?;
            str_parts.push(part.borrow_value().to_owned());
        }

        Ok(format!("{{{}}}", str_parts.join(", ")))
    }

    fn add(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.content.insert(vm, item, ())
    }

    fn remove(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.content.delete(vm, item)
    }

    fn discard(&self, item: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.content.delete_if_exists(vm, item)
    }

    fn clear(&self) {
        self.content.clear()
    }

    fn pop(&self, vm: &VirtualMachine) -> PyResult {
        // TODO: should be pop_front, but that requires rearranging every index
        if let Some((key, _)) = self.content.pop_back() {
            Ok(key)
        } else {
            let err_msg = vm.ctx.new_str("pop from an empty set");
            Err(vm.new_key_error(err_msg))
        }
    }

    fn update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        for iterable in others {
            for item in iterable.iter(vm)? {
                self.add(item?, vm)?;
            }
        }
        Ok(())
    }

    fn intersection_update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        let mut temp_inner = self.copy();
        self.clear();
        for iterable in others {
            for item in iterable.iter(vm)? {
                let obj = item?;
                if temp_inner.contains(&obj, vm)? {
                    self.add(obj, vm)?;
                }
            }
            temp_inner = self.copy()
        }
        Ok(())
    }

    fn difference_update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        for iterable in others {
            for item in iterable.iter(vm)? {
                self.content.delete_if_exists(vm, &item?)?;
            }
        }
        Ok(())
    }

    fn symmetric_difference_update(
        &self,
        others: Args<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        for iterable in others {
            // We want to remove duplicates in iterable
            let iterable_set = Self::new(iterable, vm)?;
            for item in iterable_set.content.keys() {
                self.content.delete_or_insert(vm, &item, ())?;
            }
        }
        Ok(())
    }

    fn hash(&self, vm: &VirtualMachine) -> PyResult<PyHash> {
        pyobject::hash_iter_unordered(self.content.keys().iter(), vm)
    }
}

fn extract_set(obj: &PyObjectRef) -> Option<&PySetInner> {
    match_class!(match obj {
        ref set @ PySet => Some(&set.inner),
        ref frozen @ PyFrozenSet => Some(&frozen.inner),
        _ => None,
    })
}

macro_rules! multi_args_set {
    ($vm:expr, $others:expr, $zelf:expr, $op:tt) => {{
        let mut res = $zelf.inner.copy();
        for other in $others {
            res = res.$op(other, $vm)?
        }
        Ok(Self { inner: res })
    }};
}

#[pyimpl(with(Hashable, Comparable, Iterable), flags(BASETYPE))]
impl PySet {
    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        iterable: OptionalArg<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        Self {
            inner: PySetInner::from_arg(iterable, vm)?,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__len__")]
    fn len(&self) -> usize {
        self.inner.len()
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self) -> usize {
        std::mem::size_of::<Self>() + self.inner.sizeof()
    }

    #[pymethod]
    fn copy(&self) -> Self {
        Self {
            inner: self.inner.copy(),
        }
    }

    #[pymethod(name = "__contains__")]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.contains(&needle, vm)
    }

    #[pymethod]
    fn union(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_set!(vm, others, self, union)
    }

    #[pymethod]
    fn intersection(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_set!(vm, others, self, intersection)
    }

    #[pymethod]
    fn difference(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_set!(vm, others, self, difference)
    }

    #[pymethod]
    fn symmetric_difference(
        &self,
        others: Args<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        multi_args_set!(vm, others, self, symmetric_difference)
    }

    #[pymethod]
    fn issubset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issubset(other, vm)
    }

    #[pymethod]
    fn issuperset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issuperset(other, vm)
    }

    #[pymethod]
    fn isdisjoint(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.isdisjoint(other, vm)
    }

    #[pymethod(name = "__or__")]
    fn or(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.union(other.iterable, vm)
    }

    #[pymethod(name = "__ror__")]
    fn ror(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.or(other, vm)
    }

    #[pymethod(name = "__and__")]
    fn and(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.intersection(other.iterable, vm)
    }

    #[pymethod(name = "__rand__")]
    fn rand(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.and(other, vm)
    }

    #[pymethod(name = "__sub__")]
    fn sub(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.difference(other.iterable, vm)
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.sub(other, vm)
    }

    #[pymethod(name = "__xor__")]
    fn xor(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.symmetric_difference(other.iterable, vm)
    }

    #[pymethod(name = "__rxor__")]
    fn rxor(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.xor(other, vm)
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let s = if zelf.inner.len() == 0 {
            "set()".to_owned()
        } else if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            zelf.inner.repr(vm)?
        } else {
            "set(...)".to_owned()
        };
        Ok(vm.ctx.new_str(s))
    }

    #[pymethod]
    pub fn add(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.add(item, vm)?;
        Ok(())
    }

    #[pymethod]
    fn remove(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.remove(item, vm)
    }

    #[pymethod]
    fn discard(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.discard(&item, vm)?;
        Ok(())
    }

    #[pymethod]
    fn clear(&self) {
        self.inner.clear()
    }

    #[pymethod]
    fn pop(&self, vm: &VirtualMachine) -> PyResult {
        self.inner.pop(vm)
    }

    #[pymethod(name = "__ior__")]
    fn ior(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner.update(iterable.iterable, vm)?;
        Ok(zelf.as_object().clone())
    }

    #[pymethod]
    fn update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.update(others, vm)?;
        Ok(())
    }

    #[pymethod]
    fn intersection_update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.intersection_update(others, vm)?;
        Ok(())
    }

    #[pymethod(name = "__iand__")]
    fn iand(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner.intersection_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().clone())
    }

    #[pymethod]
    fn difference_update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.difference_update(others, vm)?;
        Ok(())
    }

    #[pymethod(name = "__isub__")]
    fn isub(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner.difference_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().clone())
    }

    #[pymethod]
    fn symmetric_difference_update(
        &self,
        others: Args<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.inner.symmetric_difference_update(others, vm)?;
        Ok(())
    }

    #[pymethod(name = "__ixor__")]
    fn ixor(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner
            .symmetric_difference_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().clone())
    }
}

impl Comparable for PySet {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        extract_set(other).map_or(Ok(PyComparisonValue::NotImplemented), |other| {
            Ok(zelf.inner.compare(other, op, vm)?.into())
        })
    }
}

impl Unhashable for PySet {}

impl Iterable for PySet {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(zelf.inner.iter().into_object(vm))
    }
}

macro_rules! multi_args_frozenset {
    ($vm:expr, $others:expr, $zelf:expr, $op:tt) => {{
        let mut res = $zelf.inner.copy();
        for other in $others {
            res = res.$op(other, $vm)?
        }
        Ok(Self { inner: res })
    }};
}

#[pyimpl(flags(BASETYPE), with(Hashable, Comparable, Iterable))]
impl PyFrozenSet {
    // used by ssl.rs windows
    #[allow(dead_code)]
    pub(crate) fn from_iter(
        vm: &VirtualMachine,
        it: impl IntoIterator<Item = PyObjectRef>,
    ) -> PyResult<Self> {
        let inner = PySetInner::default();
        for elem in it {
            inner.add(elem, vm)?;
        }
        Ok(Self { inner })
    }

    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        iterable: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let iterable = if let Present(iterable) = iterable {
            if cls.is(&vm.ctx.types.frozenset_type) {
                match iterable.downcast_exact::<PyFrozenSet>(vm) {
                    Ok(iter) => return Ok(iter),
                    Err(iterable) => Present(PyIterable::try_from_object(vm, iterable)?),
                }
            } else {
                Present(PyIterable::try_from_object(vm, iterable)?)
            }
        } else {
            Missing
        };

        Self {
            inner: PySetInner::from_arg(iterable, vm)?,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__len__")]
    fn len(&self) -> usize {
        self.inner.len()
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self) -> usize {
        std::mem::size_of::<Self>() + self.inner.sizeof()
    }

    #[pymethod]
    fn copy(&self) -> Self {
        Self {
            inner: self.inner.copy(),
        }
    }

    #[pymethod(name = "__contains__")]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.contains(&needle, vm)
    }

    #[pymethod]
    fn union(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, union)
    }

    #[pymethod]
    fn intersection(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, intersection)
    }

    #[pymethod]
    fn difference(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, difference)
    }

    #[pymethod]
    fn symmetric_difference(
        &self,
        others: Args<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, symmetric_difference)
    }

    #[pymethod]
    fn issubset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issubset(other, vm)
    }

    #[pymethod]
    fn issuperset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issuperset(other, vm)
    }

    #[pymethod]
    fn isdisjoint(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.isdisjoint(other, vm)
    }

    #[pymethod(name = "__or__")]
    fn or(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.union(other.iterable, vm)
    }

    #[pymethod(name = "__ror__")]
    fn ror(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.or(other, vm)
    }

    #[pymethod(name = "__and__")]
    fn and(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.intersection(other.iterable, vm)
    }

    #[pymethod(name = "__rand__")]
    fn rand(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.and(other, vm)
    }

    #[pymethod(name = "__sub__")]
    fn sub(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.difference(other.iterable, vm)
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.sub(other, vm)
    }

    #[pymethod(name = "__xor__")]
    fn xor(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.symmetric_difference(other.iterable, vm)
    }

    #[pymethod(name = "__rxor__")]
    fn rxor(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.xor(other, vm)
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let inner = &zelf.inner;
        let s = if inner.len() == 0 {
            "frozenset()".to_owned()
        } else if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            format!("frozenset({})", inner.repr(vm)?)
        } else {
            "frozenset(...)".to_owned()
        };
        Ok(vm.ctx.new_str(s))
    }
}

impl Hashable for PyFrozenSet {
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        zelf.inner.hash(vm)
    }
}

impl Comparable for PyFrozenSet {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        extract_set(other).map_or(Ok(PyComparisonValue::NotImplemented), |other| {
            Ok(zelf.inner.compare(other, op, vm)?.into())
        })
    }
}

impl Iterable for PyFrozenSet {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(zelf.inner.iter().into_object(vm))
    }
}

struct SetIterable {
    iterable: Args<PyIterable>,
}

impl TryFromObject for SetIterable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = obj.class();
        if class.issubclass(&vm.ctx.types.set_type)
            || class.issubclass(&vm.ctx.types.frozenset_type)
        {
            // the class lease needs to be drop to be able to return the object
            drop(class);
            Ok(SetIterable {
                iterable: Args::new(vec![PyIterable::try_from_object(vm, obj)?]),
            })
        } else {
            Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", class)))
        }
    }
}

#[derive(Copy, Clone, Default)]
struct SetSizeInfo {
    size: Option<usize>,
    position: usize,
}

#[pyclass(module = false, name = "set_iterator")]
pub(crate) struct PySetIterator {
    dict: PyRc<SetContentType>,
    size_info: AtomicCell<SetSizeInfo>,
}

impl fmt::Debug for PySetIterator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("set_iterator")
    }
}

impl PyValue for PySetIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.set_iterator_type
    }
}

#[pyimpl(with(PyIter))]
impl PySetIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        let set_len = self.dict.len();
        let position = self.size_info.load().position;
        set_len.saturating_sub(position)
    }
}

impl PyIter for PySetIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let mut size_info = zelf.size_info.take();

        if let Some(set_size) = size_info.size {
            if set_size == zelf.dict.len() {
                let index = size_info.position;
                let keys = zelf.dict.keys();
                let item = keys.get(index).ok_or_else(|| vm.new_stop_iteration())?;
                size_info.position += 1;
                zelf.size_info.store(size_info);
                return Ok(item.clone());
            } else {
                size_info.size = None;
                zelf.size_info.store(size_info);
            }
        }

        Err(vm.new_runtime_error("set changed size during iteration".into()))
    }
}

pub fn init(context: &PyContext) {
    PySet::extend_class(context, &context.types.set_type);
    PyFrozenSet::extend_class(context, &context.types.frozenset_type);
    PySetIterator::extend_class(context, &context.types.set_iterator_type);
}
