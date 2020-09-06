use std::fmt;
use std::iter::FromIterator;
use std::mem::size_of;
use std::ops::DerefMut;

use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;

use super::objbool;
use super::objint::PyIntRef;
use super::objiter;
use super::objsequence::{get_item, get_pos, PySliceableSequenceMut, SequenceIndex};
use super::objslice::PySliceRef;
use super::objtype::PyClassRef;
use crate::bytesinner;
use crate::common::cell::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::function::OptionalArg;
use crate::pyobject::{
    BorrowValue, IdProtocol, PyArithmaticValue::*, PyClassImpl, PyComparisonValue, PyContext,
    PyIterable, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::sequence::{self, SimpleSeq};
use crate::vm::{ReprGuard, VirtualMachine};

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
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.list_type.clone()
    }
}

impl<'a> BorrowValue<'a> for PyList {
    type Borrowed = PyRwLockReadGuard<'a, Vec<PyObjectRef>>;

    fn borrow_value(&'a self) -> Self::Borrowed {
        self.elements.read()
    }
}

impl PyList {
    pub fn borrow_value_mut(&self) -> PyRwLockWriteGuard<'_, Vec<PyObjectRef>> {
        self.elements.write()
    }

    pub(crate) fn to_byte_inner(&self, vm: &VirtualMachine) -> PyResult<bytesinner::PyBytesInner> {
        let mut elements = Vec::<u8>::with_capacity(self.borrow_value().len());
        for elem in self.borrow_value().iter() {
            let py_int = PyIntRef::try_from_object(vm, elem.clone()).map_err(|_| {
                vm.new_type_error(format!(
                    "'{}' object cannot be interpreted as an integer",
                    elem.class().name
                ))
            })?;
            let result = py_int
                .borrow_value()
                .to_u8()
                .ok_or_else(|| vm.new_value_error("bytes must be in range (0, 256)".to_owned()))?;
            elements.push(result);
        }
        Ok(elements.into())
    }
}

#[derive(FromArgs, Default)]
pub(crate) struct SortOptions {
    #[pyarg(keyword_only, default = "None")]
    key: Option<PyObjectRef>,
    #[pyarg(keyword_only, default = "false")]
    reverse: bool,
}

pub type PyListRef = PyRef<PyList>;

#[pyimpl(flags(BASETYPE))]
impl PyList {
    #[pymethod]
    pub(crate) fn append(&self, x: PyObjectRef) {
        self.borrow_value_mut().push(x);
    }

    #[pymethod]
    fn extend(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut new_elements = vm.extract_elements(&x)?;
        self.borrow_value_mut().append(&mut new_elements);
        Ok(())
    }

    #[pymethod]
    pub(crate) fn insert(&self, position: isize, element: PyObjectRef) {
        let mut elements = self.borrow_value_mut();
        let vec_len = elements.len().to_isize().unwrap();
        // This unbounded position can be < 0 or > vec.len()
        let unbounded_position = if position < 0 {
            vec_len + position
        } else {
            position
        };
        // Bound it by [0, vec.len()]
        let position = unbounded_position.min(vec_len).to_usize().unwrap_or(0);
        elements.insert(position, element);
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(other) = other.payload_if_subclass::<PyList>(vm) {
            let e1 = self.borrow_value().clone();
            let e2 = other.borrow_value().clone();
            let elements = e1.iter().chain(e2.iter()).cloned().collect();
            Ok(vm.ctx.new_list(elements))
        } else {
            Err(vm.new_type_error(format!(
                "Cannot add {} and {}",
                Self::class(vm).name,
                other.lease_class().name
            )))
        }
    }

    #[pymethod(name = "__iadd__")]
    fn iadd(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(new_elements) = vm.extract_elements(&other) {
            let mut e = new_elements;
            zelf.borrow_value_mut().append(&mut e);
            Ok(zelf.into_object())
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self) -> bool {
        !self.borrow_value().is_empty()
    }

    #[pymethod]
    fn clear(&self) {
        let _removed = std::mem::replace(self.borrow_value_mut().deref_mut(), Vec::new());
    }

    #[pymethod]
    fn copy(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(self.borrow_value().clone())
    }

    #[pymethod(name = "__len__")]
    fn len(&self) -> usize {
        self.borrow_value().len()
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.borrow_value().capacity() * size_of::<PyObjectRef>()
    }

    #[pymethod]
    fn reverse(&self) {
        self.borrow_value_mut().reverse();
    }

    #[pymethod(name = "__reversed__")]
    fn reversed(zelf: PyRef<Self>) -> PyListReverseIterator {
        let final_position = zelf.borrow_value().len();
        PyListReverseIterator {
            position: AtomicCell::new(final_position as isize),
            list: zelf,
        }
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        get_item(vm, zelf.as_object(), &zelf.borrow_value(), needle)
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyListIterator {
        PyListIterator {
            position: AtomicCell::new(0),
            list: zelf,
        }
    }

    #[pymethod(name = "__setitem__")]
    fn setitem(
        &self,
        subscript: SequenceIndex,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match subscript {
            SequenceIndex::Int(index) => self.setindex(index, value, vm),
            SequenceIndex::Slice(slice) => {
                if let Ok(sec) = PyIterable::try_from_object(vm, value) {
                    return self.setslice(slice, sec, vm);
                }
                Err(vm.new_type_error("can only assign an iterable to a slice".to_owned()))
            }
        }
    }

    fn setindex(&self, index: isize, mut value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut elements = self.borrow_value_mut();
        if let Some(pos_index) = get_pos(index, elements.len()) {
            std::mem::swap(&mut elements[pos_index], &mut value);
            Ok(())
        } else {
            Err(vm.new_index_error("list assignment index out of range".to_owned()))
        }
    }

    fn setslice(&self, slice: PySliceRef, sec: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
        let items: Result<Vec<PyObjectRef>, _> = sec.iter(vm)?.collect();
        let items = items?;
        let mut elements = self.borrow_value_mut();
        elements.set_slice_items(vm, &slice, items.as_slice())
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let elements = zelf.borrow_value().clone();
            let mut str_parts = Vec::with_capacity(elements.len());
            for elem in elements.iter() {
                let s = vm.to_repr(elem)?;
                str_parts.push(s.borrow_value().to_owned());
            }
            format!("[{}]", str_parts.join(", "))
        } else {
            "[...]".to_owned()
        };
        Ok(s)
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("unhashable type".to_owned()))
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        let new_elements = sequence::seq_mul(&*self.borrow_value(), counter)
            .cloned()
            .collect();
        vm.ctx.new_list(new_elements)
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        self.mul(counter, &vm)
    }

    #[pymethod(name = "__imul__")]
    fn imul(zelf: PyRef<Self>, counter: isize) -> PyRef<Self> {
        let mut elements = zelf.borrow_value_mut();
        let mut new_elements: Vec<PyObjectRef> =
            sequence::seq_mul(&*elements, counter).cloned().collect();
        std::mem::swap(elements.deref_mut(), &mut new_elements);
        zelf.clone()
    }

    #[pymethod]
    fn count(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self.borrow_value().clone().iter() {
            if vm.identical_or_equal(element, &needle)? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[pymethod(name = "__contains__")]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        for element in self.borrow_value().clone().iter() {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    #[pymethod]
    fn index(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        for (index, element) in self.borrow_value().clone().iter().enumerate() {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(index);
            }
        }
        let needle_str = vm.to_str(&needle)?;
        Err(vm.new_value_error(format!("'{}' is not in list", needle_str.borrow_value())))
    }

    #[pymethod]
    fn pop(&self, i: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        let mut i = i.into_option().unwrap_or(-1);
        let mut elements = self.borrow_value_mut();
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
        let mut ri: Option<usize> = None;
        for (index, element) in self.borrow_value().clone().iter().enumerate() {
            if vm.identical_or_equal(element, &needle)? {
                ri = Some(index);
                break;
            }
        }

        if let Some(index) = ri {
            // defer delete out of borrow
            Ok(self.borrow_value_mut().remove(index))
        } else {
            let needle_str = vm.to_str(&needle)?;
            Err(vm.new_value_error(format!("'{}' is not in list", needle_str.borrow_value())))
        }
        .map(drop)
    }

    #[inline]
    fn cmp<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyResult<PyComparisonValue>
    where
        F: Fn(sequence::DynPyIter, sequence::DynPyIter) -> PyResult<bool>,
    {
        let r = if let Some(other) = other.payload_if_subclass::<PyList>(vm) {
            Implemented(op(
                self.borrow_value().boxed_iter(),
                other.borrow_value().boxed_iter(),
            )?)
        } else {
            NotImplemented
        };
        Ok(r)
    }

    #[pymethod(name = "__eq__")]
    fn eq(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if zelf.as_object().is(&other) {
            Ok(Implemented(true))
        } else {
            zelf.cmp(other, |a, b| sequence::eq(vm, a, b), vm)
        }
    }

    #[pymethod(name = "__ne__")]
    fn ne(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Ok(PyList::eq(zelf, other, vm)?.map(|v| !v))
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        self.cmp(other, |a, b| sequence::lt(vm, a, b), vm)
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        self.cmp(other, |a, b| sequence::gt(vm, a, b), vm)
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        self.cmp(other, |a, b| sequence::ge(vm, a, b), vm)
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        self.cmp(other, |a, b| sequence::le(vm, a, b), vm)
    }

    #[pymethod(name = "__delitem__")]
    fn delitem(&self, subscript: SequenceIndex, vm: &VirtualMachine) -> PyResult<()> {
        match subscript {
            SequenceIndex::Int(index) => self.delindex(index, vm),
            SequenceIndex::Slice(slice) => self.delslice(slice, vm),
        }
    }

    fn delindex(&self, index: isize, vm: &VirtualMachine) -> PyResult<()> {
        let removed = {
            let mut elements = self.borrow_value_mut();
            if let Some(pos_index) = get_pos(index, elements.len()) {
                // defer delete out of borrow
                Ok(elements.remove(pos_index))
            } else {
                Err(vm.new_index_error("Index out of bounds!".to_owned()))
            }
        };
        removed.map(drop)
    }

    fn delslice(&self, slice: PySliceRef, vm: &VirtualMachine) -> PyResult<()> {
        self.borrow_value_mut().delete_slice(vm, &slice)
    }

    #[pymethod]
    pub(crate) fn sort(&self, options: SortOptions, vm: &VirtualMachine) -> PyResult<()> {
        // replace list contents with [] for duration of sort.
        // this prevents keyfunc from messing with the list and makes it easy to
        // check if it tries to append elements to it.
        let mut elements = std::mem::take(self.borrow_value_mut().deref_mut());
        let res = do_sort(vm, &mut elements, options.key, options.reverse);
        std::mem::swap(self.borrow_value_mut().deref_mut(), &mut elements);
        res?;

        if !elements.is_empty() {
            return Err(vm.new_value_error("list modified during sort".to_owned()));
        }

        Ok(())
    }

    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        iterable: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyListRef> {
        let elements = if let OptionalArg::Present(iterable) = iterable {
            vm.extract_elements(&iterable)?
        } else {
            vec![]
        };

        PyList::from(elements).into_ref_with_type(vm, cls)
    }
}

fn do_sort(
    vm: &VirtualMachine,
    values: &mut Vec<PyObjectRef>,
    key_func: Option<PyObjectRef>,
    reverse: bool,
) -> PyResult<()> {
    let cmp = if reverse {
        VirtualMachine::_lt
    } else {
        VirtualMachine::_gt
    };
    let cmp =
        |a: &PyObjectRef, b: &PyObjectRef| objbool::boolval(vm, cmp(vm, a.clone(), b.clone())?);

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
    pub position: AtomicCell<usize>,
    pub list: PyListRef,
}

impl PyValue for PyListIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.list_iterator_type.clone()
    }
}

#[pyimpl]
impl PyListIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let list = self.list.borrow_value();
        let pos = self.position.fetch_add(1);
        if let Some(obj) = list.get(pos) {
            Ok(obj.clone())
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__length_hint__")]
    fn length_hint(&self) -> usize {
        let list = self.list.borrow_value();
        let pos = self.position.load();
        list.len().saturating_sub(pos)
    }
}

#[pyclass(module = false, name = "list_reverseiterator")]
#[derive(Debug)]
pub struct PyListReverseIterator {
    pub position: AtomicCell<isize>,
    pub list: PyListRef,
}

impl PyValue for PyListReverseIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.list_reverseiterator_type.clone()
    }
}

#[pyimpl]
impl PyListReverseIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let list = self.list.borrow_value();
        let pos = self.position.fetch_sub(1);
        if pos > 0 {
            if let Some(ret) = list.get(pos as usize - 1) {
                return Ok(ret.clone());
            }
        }
        Err(objiter::new_stop_iteration(vm))
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__length_hint__")]
    fn length_hint(&self) -> usize {
        std::cmp::max(self.position.load(), 0) as usize
    }
}

pub fn init(context: &PyContext) {
    let list_type = &context.types.list_type;
    PyList::extend_class(context, list_type);

    PyListIterator::extend_class(context, &context.types.list_iterator_type);
    PyListReverseIterator::extend_class(context, &context.types.list_reverseiterator_type);
}
