use crossbeam_utils::atomic::AtomicCell;
use std::fmt;
use std::marker::PhantomData;

use super::pytype::PyTypeRef;
use crate::common::hash::PyHash;
use crate::function::OptionalArg;
use crate::pyobject::{
    self, BorrowValue, Either, IdProtocol, IntoPyObject, PyArithmaticValue, PyClassImpl,
    PyComparisonValue, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TransmuteFromObject,
    TryFromObject, TypeProtocol,
};
use crate::sequence::{self, SimpleSeq};
use crate::sliceable::PySliceableSequence;
use crate::slots::{Comparable, Hashable, Iterable, PyComparisonOp, PyIter};
use crate::vm::{ReprGuard, VirtualMachine};

/// tuple() -> empty tuple
/// tuple(iterable) -> tuple initialized from iterable's items
///
/// If the argument is a tuple, the return value is the same object.
#[pyclass(module = false, name = "tuple")]
pub struct PyTuple {
    elements: Box<[PyObjectRef]>,
}

impl fmt::Debug for PyTuple {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more informational, non-recursive Debug formatter
        f.write_str("tuple")
    }
}

impl<'a> BorrowValue<'a> for PyTuple {
    type Borrowed = &'a [PyObjectRef];

    fn borrow_value(&'a self) -> Self::Borrowed {
        &self.elements
    }
}

impl PyValue for PyTuple {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.tuple_type
    }
}

macro_rules! impl_intopyobj_tuple {
    ($(($T:ident, $idx:tt)),+) => {
        impl<$($T: IntoPyObject),*> IntoPyObject for ($($T,)*) {
            fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
                vm.ctx.new_tuple(vec![$(self.$idx.into_pyobject(vm)),*])
            }
        }
    };
}

impl_intopyobj_tuple!((A, 0));
impl_intopyobj_tuple!((A, 0), (B, 1));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6));

impl PyTuple {
    pub(crate) fn fast_getitem(&self, idx: usize) -> PyObjectRef {
        self.elements[idx].clone()
    }
}

pub type PyTupleRef = PyRef<PyTuple>;

impl PyTupleRef {
    pub(crate) fn with_elements(elements: Vec<PyObjectRef>, ctx: &PyContext) -> Self {
        if elements.is_empty() {
            ctx.empty_tuple.clone()
        } else {
            let elements = elements.into_boxed_slice();
            Self::new_ref(PyTuple { elements }, ctx.types.tuple_type.clone(), None)
        }
    }
}

#[pyimpl(flags(BASETYPE), with(Hashable, Comparable, Iterable))]
impl PyTuple {
    /// Creating a new tuple with given boxed slice.
    /// NOTE: for usual case, you probably want to use PyTupleRef::with_elements.
    /// Calling this function implies trying micro optimization for non-zero-sized tuple.
    pub(crate) fn _new(elements: Box<[PyObjectRef]>) -> Self {
        Self { elements }
    }

    #[pymethod(name = "__add__")]
    fn add(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyArithmaticValue<PyRef<Self>> {
        let added = other.downcast::<Self>().map(|other| {
            if other.elements.is_empty() && zelf.class().is(&vm.ctx.types.tuple_type) {
                zelf
            } else if zelf.elements.is_empty() && other.class().is(&vm.ctx.types.tuple_type) {
                other
            } else {
                let elements = zelf
                    .elements
                    .boxed_iter()
                    .chain(other.borrow_value().boxed_iter())
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                Self { elements }.into_ref(vm)
            }
        });
        PyArithmaticValue::from_option(added.ok())
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self) -> bool {
        !self.elements.is_empty()
    }

    #[pymethod(name = "count")]
    fn count(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self.elements.iter() {
            if vm.identical_or_equal(element, &needle)? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[pymethod(name = "__len__")]
    pub(crate) fn len(&self) -> usize {
        self.elements.len()
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let mut str_parts = Vec::with_capacity(zelf.elements.len());
            for elem in zelf.elements.iter() {
                let s = vm.to_repr(elem)?;
                str_parts.push(s.borrow_value().to_owned());
            }

            if str_parts.len() == 1 {
                format!("({},)", str_parts[0])
            } else {
                format!("({})", str_parts.join(", "))
            }
        } else {
            "(...)".to_owned()
        };
        Ok(s)
    }

    #[pymethod(name = "__mul__")]
    #[pymethod(name = "__rmul__")]
    fn mul(&self, counter: isize, vm: &VirtualMachine) -> PyRef<Self> {
        if self.elements.is_empty() || counter == 0 {
            vm.ctx.empty_tuple.clone()
        } else {
            let elements = sequence::seq_mul(&self.elements, counter)
                .cloned()
                .collect::<Vec<_>>()
                .into_boxed_slice();
            Self { elements }.into_ref(vm)
        }
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let result = match zelf.elements.as_ref().get_item(vm, needle, "tuple")? {
            Either::A(obj) => obj,
            Either::B(vec) => vm.ctx.new_tuple(vec),
        };
        Ok(result)
    }

    #[pymethod(name = "index")]
    fn index(
        &self,
        needle: PyObjectRef,
        start: OptionalArg<isize>,
        stop: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let mut start = start.into_option().unwrap_or(0);
        if start < 0 {
            start += self.borrow_value().len() as isize;
            if start < 0 {
                start = 0;
            }
        }
        let mut stop = stop.into_option().unwrap_or(isize::MAX);
        if stop < 0 {
            stop += self.borrow_value().len() as isize;
            if stop < 0 {
                stop = 0;
            }
        }
        for (index, element) in self
            .elements
            .iter()
            .enumerate()
            .take(stop as usize)
            .skip(start as usize)
        {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(index);
            }
        }
        Err(vm.new_value_error("tuple.index(x): x not in tuple".to_owned()))
    }

    #[pymethod(name = "__contains__")]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        for element in self.elements.iter() {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[pymethod(magic)]
    fn getnewargs(zelf: PyRef<Self>, vm: &VirtualMachine) -> (PyTupleRef,) {
        // the arguments to pass to tuple() is just one tuple - so we'll be doing tuple(tup), which
        // should just return tup, or tuplesubclass(tup), which'll copy/validate (e.g. for a
        // structseq)
        let tup_arg = if zelf.class().is(&vm.ctx.types.tuple_type) {
            zelf
        } else {
            PyTupleRef::with_elements(zelf.elements.clone().into_vec(), &vm.ctx)
        };
        (tup_arg,)
    }

    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        iterable: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let elements = if let OptionalArg::Present(iterable) = iterable {
            let iterable = if cls.is(&vm.ctx.types.tuple_type) {
                match iterable.downcast_exact::<Self>(vm) {
                    Ok(tuple) => return Ok(tuple),
                    Err(iterable) => iterable,
                }
            } else {
                iterable
            };
            vm.extract_elements(&iterable)?
        } else {
            vec![]
        }
        .into_boxed_slice();

        Self { elements }.into_ref_with_type(vm, cls)
    }
}

impl Hashable for PyTuple {
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        pyobject::hash_iter(zelf.elements.iter(), vm)
    }
}

impl Comparable for PyTuple {
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
        let a = zelf.borrow_value();
        let b = other.borrow_value();
        sequence::cmp(vm, a.boxed_iter(), b.boxed_iter(), op).map(PyComparisonValue::Implemented)
    }
}

impl Iterable for PyTuple {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyTupleIterator {
            position: AtomicCell::new(0),
            tuple: zelf,
        }
        .into_object(vm))
    }
}

#[pyclass(module = false, name = "tuple_iterator")]
#[derive(Debug)]
pub(crate) struct PyTupleIterator {
    position: AtomicCell<usize>,
    tuple: PyTupleRef,
}

impl PyValue for PyTupleIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.tuple_iterator_type
    }
}

#[pyimpl(with(PyIter))]
impl PyTupleIterator {}

impl PyIter for PyTupleIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let pos = zelf.position.fetch_add(1);
        if let Some(obj) = zelf.tuple.borrow_value().get(pos) {
            Ok(obj.clone())
        } else {
            Err(vm.new_stop_iteration())
        }
    }
}

pub(crate) fn init(context: &PyContext) {
    PyTuple::extend_class(context, &context.types.tuple_type);
    PyTupleIterator::extend_class(context, &context.types.tuple_iterator_type);
}

pub struct PyTupleTyped<T: TransmuteFromObject> {
    // SAFETY INVARIANT: T must be repr(transparent) over PyObjectRef, and the
    //                   elements must be logically valid when transmuted to T
    tuple: PyTupleRef,
    _marker: PhantomData<Vec<T>>,
}

impl<T: TransmuteFromObject> TryFromObject for PyTupleTyped<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let tuple = PyTupleRef::try_from_object(vm, obj)?;
        for elem in tuple.borrow_value() {
            T::check(vm, elem)?
        }
        // SAFETY: the contract of TransmuteFromObject upholds the variant on `tuple`
        Ok(Self {
            tuple,
            _marker: PhantomData,
        })
    }
}

impl<T: TransmuteFromObject> Clone for PyTupleTyped<T> {
    fn clone(&self) -> Self {
        Self {
            tuple: self.tuple.clone(),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: TransmuteFromObject + 'a> BorrowValue<'a> for PyTupleTyped<T> {
    type Borrowed = &'a [T];
    #[inline]
    fn borrow_value(&'a self) -> Self::Borrowed {
        unsafe { &*(self.tuple.borrow_value() as *const [PyObjectRef] as *const [T]) }
    }
}

impl<T: TransmuteFromObject + fmt::Debug> fmt::Debug for PyTupleTyped<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.borrow_value().fmt(f)
    }
}

impl<T: TransmuteFromObject> From<PyTupleTyped<T>> for PyTupleRef {
    #[inline]
    fn from(tup: PyTupleTyped<T>) -> Self {
        tup.tuple
    }
}

impl<T: TransmuteFromObject> IntoPyObject for PyTupleTyped<T> {
    #[inline]
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.tuple.into_object()
    }
}
