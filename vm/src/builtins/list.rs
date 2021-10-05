use super::{PositionIterInternal, PyGenericAlias, PySliceRef, PyTypeRef};
use crate::common::lock::{
    PyMappedRwLockReadGuard, PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard,
};
use crate::{
    function::{ArgIterable, FuncArgs, OptionalArg},
    protocol::{PyIterReturn, PyMappingMethods},
    sequence::{self, SimpleSeq},
    sliceable::{PySliceableSequence, PySliceableSequenceMut, SequenceIndex},
    slots::{
        AsMapping, Comparable, Hashable, Iterable, IteratorIterable, PyComparisonOp, SlotIterator,
        Unhashable,
    },
    stdlib::sys,
    utils::Either,
    vm::{ReprGuard, VirtualMachine},
    PyClassDef, PyClassImpl, PyComparisonValue, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use std::fmt;
use std::iter::FromIterator;
use std::mem::size_of;
use std::ops::DerefMut;

/// Built-in mutable sequence.
///
/// If no argument is given, the constructor creates a new empty list.
/// The argument must be an iterable if specified.
#[pyclass(module = false, name = "list")]
#[derive(Default)]
pub struct PyList {
    elements: PyRwLock<Vec<PyObjectRef>>,
}

impl fmt::Debug for PyList {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("list")
    }
}

impl From<Vec<PyObjectRef>> for PyList {
    fn from(elements: Vec<PyObjectRef>) -> Self {
        PyList {
            elements: PyRwLock::new(elements),
        }
    }
}

impl FromIterator<PyObjectRef> for PyList {
    fn from_iter<T: IntoIterator<Item = PyObjectRef>>(iter: T) -> Self {
        Vec::from_iter(iter).into()
    }
}

impl PyValue for PyList {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.list_type
    }
}

impl PyList {
    pub fn borrow_vec(&self) -> PyMappedRwLockReadGuard<'_, [PyObjectRef]> {
        PyRwLockReadGuard::map(self.elements.read(), |v| &**v)
    }

    pub fn borrow_vec_mut(&self) -> PyRwLockWriteGuard<'_, Vec<PyObjectRef>> {
        self.elements.write()
    }
}

#[derive(FromArgs, Default)]
pub(crate) struct SortOptions {
    #[pyarg(named, default)]
    key: Option<PyObjectRef>,
    #[pyarg(named, default = "false")]
    reverse: bool,
}

pub type PyListRef = PyRef<PyList>;

#[pyimpl(with(AsMapping, Iterable, Hashable, Comparable), flags(BASETYPE))]
impl PyList {
    #[pymethod]
    pub(crate) fn append(&self, x: PyObjectRef) {
        self.borrow_vec_mut().push(x);
    }

    #[pymethod]
    fn extend(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut new_elements = vm.extract_elements(&x)?;
        self.borrow_vec_mut().append(&mut new_elements);
        Ok(())
    }

    #[pymethod]
    pub(crate) fn insert(&self, position: isize, element: PyObjectRef) {
        let mut elements = self.borrow_vec_mut();
        let position = elements.saturate_index(position);
        elements.insert(position, element);
    }

    #[pymethod(magic)]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(other) = other.payload_if_subclass::<PyList>(vm) {
            let mut elements = self.borrow_vec().to_vec();
            elements.extend(other.borrow_vec().iter().cloned());
            Ok(vm.ctx.new_list(elements))
        } else {
            Err(vm.new_type_error(format!(
                "Cannot add {} and {}",
                Self::class(vm).name(),
                other.class().name()
            )))
        }
    }

    #[pymethod(magic)]
    fn iadd(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if let Ok(new_elements) = vm.extract_elements(&other) {
            let mut e = new_elements;
            zelf.borrow_vec_mut().append(&mut e);
            zelf.into_object()
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.borrow_vec().is_empty()
    }

    #[pymethod]
    fn clear(&self) {
        let _removed = std::mem::take(self.borrow_vec_mut().deref_mut());
    }

    #[pymethod]
    fn copy(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(self.borrow_vec().to_vec())
    }

    #[pymethod(magic)]
    fn len(&self) -> usize {
        self.borrow_vec().len()
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.elements.read().capacity() * size_of::<PyObjectRef>()
    }

    #[pymethod]
    fn reverse(&self) {
        self.borrow_vec_mut().reverse();
    }

    #[pymethod(magic)]
    fn reversed(zelf: PyRef<Self>) -> PyListReverseIterator {
        let position = zelf.len().saturating_sub(1);
        PyListReverseIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, position)),
        }
    }

    #[pymethod(magic)]
    fn getitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let result = match zelf.borrow_vec().get_item(vm, needle, Self::NAME)? {
            Either::A(obj) => obj,
            Either::B(vec) => vm.ctx.new_list(vec),
        };
        Ok(result)
    }

    #[pymethod(magic)]
    fn setitem(
        &self,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match SequenceIndex::try_from_object_for(vm, needle, Self::NAME)? {
            SequenceIndex::Int(index) => self.setindex(index, value, vm),
            SequenceIndex::Slice(slice) => {
                if let Ok(sec) = ArgIterable::try_from_object(vm, value) {
                    return self.setslice(slice, sec, vm);
                }
                Err(vm.new_type_error("can only assign an iterable to a slice".to_owned()))
            }
        }
    }

    fn setindex(&self, index: isize, mut value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut elements = self.borrow_vec_mut();
        if let Some(pos_index) = elements.wrap_index(index) {
            std::mem::swap(&mut elements[pos_index], &mut value);
            Ok(())
        } else {
            Err(vm.new_index_error("list assignment index out of range".to_owned()))
        }
    }

    fn setslice(&self, slice: PySliceRef, sec: ArgIterable, vm: &VirtualMachine) -> PyResult<()> {
        let items: Result<Vec<PyObjectRef>, _> = sec.iter(vm)?.collect();
        let items = items?;
        let mut elements = self.borrow_vec_mut();
        elements.set_slice_items(vm, &slice, items.as_slice())
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let elements = zelf.borrow_vec().to_vec();
            let mut str_parts = Vec::with_capacity(elements.len());
            for elem in elements.iter() {
                let s = vm.to_repr(elem)?;
                str_parts.push(s.as_str().to_owned());
            }
            format!("[{}]", str_parts.join(", "))
        } else {
            "[...]".to_owned()
        };
        Ok(s)
    }

    #[pymethod(magic)]
    #[pymethod(name = "__rmul__")]
    fn mul(&self, value: isize, vm: &VirtualMachine) -> PyResult {
        let new_elements = sequence::seq_mul(vm, &self.borrow_vec(), value)?
            .cloned()
            .collect();
        Ok(vm.ctx.new_list(new_elements))
    }

    #[pymethod(magic)]
    fn imul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let mut elements = zelf.borrow_vec_mut();
        let mut new_elements: Vec<PyObjectRef> =
            sequence::seq_mul(vm, &*elements, value)?.cloned().collect();
        std::mem::swap(elements.deref_mut(), &mut new_elements);
        Ok(zelf.clone())
    }

    #[pymethod]
    fn count(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        // TODO: to_vec() cause copy which leads to cost O(N). It need to be improved.
        let elements = self.borrow_vec().to_vec();
        let mut count: usize = 0;
        for elem in elements.iter() {
            if vm.identical_or_equal(elem, &needle)? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[pymethod(magic)]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        // TODO: to_vec() cause copy which leads to cost O(N). It need to be improved.
        let elements = self.borrow_vec().to_vec();
        for elem in elements.iter() {
            if vm.identical_or_equal(elem, &needle)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    #[pymethod]
    fn index(
        &self,
        needle: PyObjectRef,
        start: OptionalArg<isize>,
        stop: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let mut start = start.into_option().unwrap_or(0);
        if start < 0 {
            start += self.borrow_vec().len() as isize;
            if start < 0 {
                start = 0;
            }
        }
        let mut stop = stop.into_option().unwrap_or(sys::MAXSIZE);
        if stop < 0 {
            stop += self.borrow_vec().len() as isize;
            if stop < 0 {
                stop = 0;
            }
        }
        // TODO: to_vec() cause copy which leads to cost O(N). It need to be improved.
        let elements = self.borrow_vec().to_vec();
        for (index, element) in elements
            .iter()
            .enumerate()
            .take(stop as usize)
            .skip(start as usize)
        {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(index);
            }
        }
        Err(vm.new_value_error(format!("'{}' is not in list", vm.to_str(&needle)?)))
    }

    #[pymethod]
    fn pop(&self, i: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        let mut i = i.into_option().unwrap_or(-1);
        let mut elements = self.borrow_vec_mut();
        if i < 0 {
            i += elements.len() as isize;
        }
        if elements.is_empty() {
            Err(vm.new_index_error("pop from empty list".to_owned()))
        } else if i < 0 || i as usize >= elements.len() {
            Err(vm.new_index_error("pop index out of range".to_owned()))
        } else {
            Ok(elements.remove(i as usize))
        }
    }

    #[pymethod]
    fn remove(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // TODO: to_vec() cause copy which leads to cost O(N). It need to be improved.
        let elements = self.borrow_vec().to_vec();
        let mut ri: Option<usize> = None;
        for (index, element) in elements.iter().enumerate() {
            if vm.identical_or_equal(element, &needle)? {
                ri = Some(index);
                break;
            }
        }

        if let Some(index) = ri {
            // defer delete out of borrow
            Ok(self.borrow_vec_mut().remove(index))
        } else {
            Err(vm.new_value_error(format!("'{}' is not in list", vm.to_str(&needle)?)))
        }
        .map(drop)
    }

    #[pymethod(magic)]
    fn delitem(&self, subscript: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        match SequenceIndex::try_from_object_for(vm, subscript, Self::NAME)? {
            SequenceIndex::Int(index) => self.delindex(index, vm),
            SequenceIndex::Slice(slice) => self.delslice(slice, vm),
        }
    }

    fn delindex(&self, index: isize, vm: &VirtualMachine) -> PyResult<()> {
        let removed = {
            let mut elements = self.borrow_vec_mut();
            if let Some(pos_index) = elements.wrap_index(index) {
                // defer delete out of borrow
                Ok(elements.remove(pos_index))
            } else {
                Err(vm.new_index_error("Index out of bounds!".to_owned()))
            }
        };
        removed.map(drop)
    }

    fn delslice(&self, slice: PySliceRef, vm: &VirtualMachine) -> PyResult<()> {
        self.borrow_vec_mut().delete_slice(vm, &slice)
    }

    #[pymethod]
    pub(crate) fn sort(&self, options: SortOptions, vm: &VirtualMachine) -> PyResult<()> {
        // replace list contents with [] for duration of sort.
        // this prevents keyfunc from messing with the list and makes it easy to
        // check if it tries to append elements to it.
        let mut elements = std::mem::take(self.borrow_vec_mut().deref_mut());
        let res = do_sort(vm, &mut elements, options.key, options.reverse);
        std::mem::swap(self.borrow_vec_mut().deref_mut(), &mut elements);
        res?;

        if !elements.is_empty() {
            return Err(vm.new_value_error("list modified during sort".to_owned()));
        }

        Ok(())
    }

    #[pyslot]
    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyList::default().into_pyresult_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn init(&self, iterable: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
        let mut elements = if let OptionalArg::Present(iterable) = iterable {
            vm.extract_elements(&iterable)?
        } else {
            vec![]
        };
        std::mem::swap(self.borrow_vec_mut().deref_mut(), &mut elements);
        Ok(())
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

impl AsMapping for PyList {
    fn as_mapping(_zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<PyMappingMethods> {
        Ok(PyMappingMethods {
            length: Some(Self::length),
            subscript: Some(Self::subscript),
            ass_subscript: Some(Self::ass_subscript),
        })
    }

    #[inline]
    fn length(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        Self::downcast_ref(&zelf, vm).map(|zelf| Ok(zelf.len()))?
    }

    #[inline]
    fn subscript(zelf: PyObjectRef, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Self::downcast(zelf, vm).map(|zelf| Self::getitem(zelf, needle, vm))?
    }

    #[inline]
    fn ass_subscript(
        zelf: PyObjectRef,
        needle: PyObjectRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        Self::downcast_ref(&zelf, vm).map(|zelf| match value {
            Some(value) => zelf.setitem(needle, value, vm),
            None => zelf.delitem(needle, vm),
        })?
    }
}

impl Iterable for PyList {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyListIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
        }
        .into_object(vm))
    }
}

impl Comparable for PyList {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(zelf, other) {
            return Ok(res.into());
        }
        let other = class_or_notimplemented!(Self, other);
        let a = zelf.borrow_vec();
        let b = other.borrow_vec();
        sequence::cmp(vm, a.boxed_iter(), b.boxed_iter(), op).map(PyComparisonValue::Implemented)
    }
}

impl Unhashable for PyList {}

fn do_sort(
    vm: &VirtualMachine,
    values: &mut Vec<PyObjectRef>,
    key_func: Option<PyObjectRef>,
    reverse: bool,
) -> PyResult<()> {
    let op = if reverse {
        PyComparisonOp::Lt
    } else {
        PyComparisonOp::Gt
    };
    let cmp = |a: &PyObjectRef, b: &PyObjectRef| vm.bool_cmp(a, b, op);

    if let Some(ref key_func) = key_func {
        let mut items = values
            .iter()
            .map(|x| Ok((x.clone(), vm.invoke(key_func, vec![x.clone()])?)))
            .collect::<Result<Vec<_>, _>>()?;
        timsort::try_sort_by_gt(&mut items, |a, b| cmp(&a.1, &b.1))?;
        *values = items.into_iter().map(|(val, _)| val).collect();
    } else {
        timsort::try_sort_by_gt(values, cmp)?;
    }

    Ok(())
}

#[pyclass(module = false, name = "list_iterator")]
#[derive(Debug)]
pub struct PyListIterator {
    internal: PyMutex<PositionIterInternal<PyListRef>>,
}

impl PyValue for PyListIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.list_iterator_type
    }
}

#[pyimpl(with(SlotIterator))]
impl PyListIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        self.internal.lock().length_hint(|obj| obj.len())
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal
            .lock()
            .set_state(state, |obj, pos| pos.min(obj.len()), vm)
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.internal
            .lock()
            .builtins_iter_reduce(|x| x.clone().into_object(), vm)
    }
}

impl IteratorIterable for PyListIterator {}
impl SlotIterator for PyListIterator {
    fn next(zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|list, pos| {
            let vec = list.borrow_vec();
            Ok(match vec.get(pos) {
                Some(x) => PyIterReturn::Return(x.clone()),
                None => PyIterReturn::StopIteration(None),
            })
        })
    }
}

#[pyclass(module = false, name = "list_reverseiterator")]
#[derive(Debug)]
pub struct PyListReverseIterator {
    internal: PyMutex<PositionIterInternal<PyListRef>>,
}

impl PyValue for PyListReverseIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.list_reverseiterator_type
    }
}

#[pyimpl(with(SlotIterator))]
impl PyListReverseIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        self.internal.lock().rev_length_hint(|obj| obj.len())
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal
            .lock()
            .set_state(state, |obj, pos| pos.min(obj.len()), vm)
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.internal
            .lock()
            .builtins_reversed_reduce(|x| x.clone().into_object(), vm)
    }
}

impl IteratorIterable for PyListReverseIterator {}
impl SlotIterator for PyListReverseIterator {
    fn next(zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().rev_next(|list, pos| {
            let vec = list.borrow_vec();
            Ok(match vec.get(pos) {
                Some(x) => PyIterReturn::Return(x.clone()),
                None => PyIterReturn::StopIteration(None),
            })
        })
    }
}

pub fn init(context: &PyContext) {
    let list_type = &context.types.list_type;
    PyList::extend_class(context, list_type);

    PyListIterator::extend_class(context, &context.types.list_iterator_type);
    PyListReverseIterator::extend_class(context, &context.types.list_reverseiterator_type);
}
