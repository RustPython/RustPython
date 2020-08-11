/*
 * Builtin set type with a sequence of unique items.
 */
use rustpython_common::rc::PyRc;
use std::fmt;

use super::objiter;
use super::objtype::{self, PyClassRef};
use crate::dictdatatype;
use crate::function::{Args, OptionalArg};
use crate::pyobject::{
    self, BorrowValue, PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::vm::{ReprGuard, VirtualMachine};
use rustpython_common::hash::PyHash;

pub type SetContentType = dictdatatype::Dict<()>;

/// set() -> new empty set object
/// set(iterable) -> new set object
///
/// Build an unordered collection of unique elements.
#[pyclass]
#[derive(Default)]
pub struct PySet {
    inner: PySetInner,
}
pub type PySetRef = PyRef<PySet>;

/// frozenset() -> empty frozenset object
/// frozenset(iterable) -> frozenset object
///
/// Build an immutable unordered collection of unique elements.
#[pyclass]
#[derive(Default)]
pub struct PyFrozenSet {
    inner: PySetInner,
}
pub type PyFrozenSetRef = PyRef<PyFrozenSet>;

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
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.set_type()
    }
}

impl PyValue for PyFrozenSet {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.frozenset_type()
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

    #[inline]
    fn _compare_inner(
        &self,
        other: &PySetInner,
        size_func: fn(usize, usize) -> bool,
        swap: bool,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let (zelf, other) = if swap { (other, self) } else { (self, other) };

        if size_func(zelf.len(), other.len()) {
            return Ok(false);
        }
        for key in other.content.keys() {
            if !zelf.contains(&key, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn eq(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult<bool> {
        self._compare_inner(
            other,
            |zelf: usize, other: usize| -> bool { zelf != other },
            false,
            vm,
        )
    }

    fn ne(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(!self.eq(other, vm)?)
    }

    fn ge(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult<bool> {
        self._compare_inner(
            other,
            |zelf: usize, other: usize| -> bool { zelf < other },
            false,
            vm,
        )
    }

    fn gt(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult<bool> {
        self._compare_inner(
            other,
            |zelf: usize, other: usize| -> bool { zelf <= other },
            false,
            vm,
        )
    }

    fn le(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult<bool> {
        self._compare_inner(
            other,
            |zelf: usize, other: usize| -> bool { zelf < other },
            true,
            vm,
        )
    }

    fn lt(&self, other: &PySetInner, vm: &VirtualMachine) -> PyResult<bool> {
        self._compare_inner(
            other,
            |zelf: usize, other: usize| -> bool { zelf <= other },
            true,
            vm,
        )
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
        self.le(&other_set, vm)
    }

    fn isdisjoint(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        for item in other.iter(vm)? {
            if self.contains(&item?, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn iter(&self, _vm: &VirtualMachine) -> PySetIterator {
        let set_size = SetSizeInfo {
            position: 0,
            size: Some(self.content.len()),
        };

        PySetIterator {
            dict: PyRc::clone(&self.content),
            size_info: crossbeam_utils::atomic::AtomicCell::new(set_size),
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
        self.content.delete(vm, &item)
    }

    fn discard(&self, item: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.content.delete_if_exists(vm, &item)
    }

    fn clear(&self) {
        self.content.clear()
    }

    fn pop(&self, vm: &VirtualMachine) -> PyResult {
        if let Some((key, _)) = self.content.pop_front() {
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

macro_rules! try_set_cmp {
    ($vm:expr, $other:expr, $op:expr) => {
        Ok(match_class!(match ($other) {
            set @ PySet => ($vm.ctx.new_bool($op(&set.inner)?)),
            frozen @ PyFrozenSet => ($vm.ctx.new_bool($op(&frozen.inner)?)),
            _ => $vm.ctx.not_implemented(),
        }));
    };
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

#[pyimpl(flags(BASETYPE))]
impl PySet {
    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        iterable: OptionalArg<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<PySetRef> {
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

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.eq(other, vm))
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.ne(other, vm))
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.ge(other, vm))
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.gt(other, vm))
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.le(other, vm))
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.lt(other, vm))
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

    #[pymethod(name = "__iter__")]
    fn iter(&self, vm: &VirtualMachine) -> PySetIterator {
        self.inner.iter(vm)
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
    fn update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult {
        self.inner.update(others, vm)?;
        Ok(vm.get_none())
    }

    #[pymethod]
    fn intersection_update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult {
        self.inner.intersection_update(others, vm)?;
        Ok(vm.get_none())
    }

    #[pymethod(name = "__iand__")]
    fn iand(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner.intersection_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().clone())
    }

    #[pymethod]
    fn difference_update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult {
        self.inner.difference_update(others, vm)?;
        Ok(vm.get_none())
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
    ) -> PyResult {
        self.inner.symmetric_difference_update(others, vm)?;
        Ok(vm.get_none())
    }

    #[pymethod(name = "__ixor__")]
    fn ixor(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner
            .symmetric_difference_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().clone())
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("unhashable type".to_owned()))
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

#[pyimpl(flags(BASETYPE))]
impl PyFrozenSet {
    pub fn from_iter(
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
        cls: PyClassRef,
        iterable: OptionalArg<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<PyFrozenSetRef> {
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

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.eq(other, vm))
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.ne(other, vm))
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.ge(other, vm))
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.gt(other, vm))
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.le(other, vm))
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_set_cmp!(vm, other, |other| self.inner.lt(other, vm))
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

    #[pymethod(name = "__iter__")]
    fn iter(&self, vm: &VirtualMachine) -> PySetIterator {
        self.inner.iter(vm)
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

    #[pymethod(name = "__hash__")]
    fn hash(&self, vm: &VirtualMachine) -> PyResult<PyHash> {
        self.inner.hash(vm)
    }
}

struct SetIterable {
    iterable: Args<PyIterable>,
}

impl TryFromObject for SetIterable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = obj.lease_class();
        if objtype::issubclass(&class, &vm.ctx.types.set_type)
            || objtype::issubclass(&class, &vm.ctx.types.frozenset_type)
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

#[pyclass]
struct PySetIterator {
    dict: PyRc<SetContentType>,
    size_info: crossbeam_utils::atomic::AtomicCell<SetSizeInfo>,
}

impl fmt::Debug for PySetIterator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("setiterator")
    }
}

#[pyimpl]
impl PySetIterator {
    #[pymethod(magic)]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let mut size_info = self.size_info.take();

        if let Some(set_size) = size_info.size {
            if set_size == self.dict.len() {
                let index = size_info.position;
                if let Some(item) = self.dict.keys().get(index) {
                    size_info.position += 1;
                    self.size_info.store(size_info);
                    return Ok(item.clone());
                } else {
                    return Err(objiter::new_stop_iteration(vm));
                }
            } else {
                size_info.size = None;
                self.size_info.store(size_info);
            }
        }

        Err(vm.new_runtime_error("set changed size during iteration".into()))
    }

    #[pymethod(magic)]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        let set_len = self.dict.len();
        let position = self.size_info.load().position;
        set_len.saturating_sub(position)
    }
}

impl PyValue for PySetIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.setiterator_type()
    }
}

pub fn init(context: &PyContext) {
    PySet::extend_class(context, &context.types.set_type);
    PyFrozenSet::extend_class(context, &context.types.frozenset_type);
    PySetIterator::extend_class(context, &context.types.setiterator_type);
}
