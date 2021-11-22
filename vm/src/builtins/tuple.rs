use super::{PositionIterInternal, PyGenericAlias, PyTypeRef};
use crate::common::hash::PyHash;
use crate::{
    function::{IntoPyObject, OptionalArg},
    protocol::{PyIterReturn, PyMappingMethods},
    sequence::{ObjectSequenceOp, SequenceOp},
    sliceable::PySliceableSequence,
    stdlib::sys,
    types::{
        AsMapping, Comparable, Constructor, Hashable, IterNext, IterNextIterable, Iterable,
        PyComparisonOp, Unconstructible,
    },
    utils::Either,
    vm::{ReprGuard, VirtualMachine},
    IdProtocol, PyArithmeticValue, PyClassDef, PyClassImpl, PyComparisonValue, PyContext, PyObject,
    PyObjectRef, PyRef, PyResult, PyValue, TransmuteFromObject, TryFromObject, TypeProtocol,
};
use rustpython_common::lock::PyMutex;
use std::fmt;
use std::marker::PhantomData;

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

impl PyValue for PyTuple {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.tuple_type
    }
}

pub trait IntoPyTuple {
    fn into_pytuple(self, vm: &VirtualMachine) -> PyTupleRef;
}

impl IntoPyTuple for () {
    fn into_pytuple(self, vm: &VirtualMachine) -> PyTupleRef {
        vm.ctx.empty_tuple.clone()
    }
}

impl IntoPyTuple for Vec<PyObjectRef> {
    fn into_pytuple(self, vm: &VirtualMachine) -> PyTupleRef {
        PyTuple::new_ref(self, &vm.ctx)
    }
}

macro_rules! impl_intopyobj_tuple {
    ($(($T:ident, $idx:tt)),+) => {
        impl<$($T: IntoPyObject),*> IntoPyTuple for ($($T,)*) {
            fn into_pytuple(self, vm: &VirtualMachine) -> PyTupleRef {
                PyTuple::new_ref(vec![$(self.$idx.into_pyobject(vm)),*], &vm.ctx)
            }
        }

        impl<$($T: IntoPyObject),*> IntoPyObject for ($($T,)*) {
            fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
                self.into_pytuple(vm).into()
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

impl Constructor for PyTuple {
    type Args = OptionalArg<PyObjectRef>;

    fn py_new(cls: PyTypeRef, iterable: Self::Args, vm: &VirtualMachine) -> PyResult {
        let elements = if let OptionalArg::Present(iterable) = iterable {
            let iterable = if cls.is(&vm.ctx.types.tuple_type) {
                match iterable.downcast_exact::<Self>(vm) {
                    Ok(tuple) => return Ok(tuple.into()),
                    Err(iterable) => iterable,
                }
            } else {
                iterable
            };
            vm.extract_elements(&iterable)?
        } else {
            vec![]
        };
        // Return empty tuple only for exact tuple types if the iterable is empty.
        if elements.is_empty() && cls.is(&vm.ctx.types.tuple_type) {
            Ok(vm.ctx.empty_tuple.clone().into())
        } else {
            Self {
                elements: elements.into_boxed_slice(),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }
}

impl PyTuple {
    pub fn new_ref(elements: Vec<PyObjectRef>, ctx: &PyContext) -> PyRef<Self> {
        if elements.is_empty() {
            ctx.empty_tuple.clone()
        } else {
            let elements = elements.into_boxed_slice();
            PyRef::new_ref(Self { elements }, ctx.types.tuple_type.clone(), None)
        }
    }

    /// Creating a new tuple with given boxed slice.
    /// NOTE: for usual case, you probably want to use PyTuple::new_ref.
    /// Calling this function implies trying micro optimization for non-zero-sized tuple.
    pub fn new_unchecked(elements: Box<[PyObjectRef]>) -> Self {
        Self { elements }
    }

    pub fn as_slice(&self) -> &[PyObjectRef] {
        &self.elements
    }
}

#[pyimpl(
    flags(BASETYPE),
    with(AsMapping, Hashable, Comparable, Iterable, Constructor)
)]
impl PyTuple {
    #[pymethod(magic)]
    fn add(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyArithmeticValue<PyRef<Self>> {
        let added = other.downcast::<Self>().map(|other| {
            if other.elements.is_empty() && zelf.class().is(&vm.ctx.types.tuple_type) {
                zelf
            } else if zelf.elements.is_empty() && other.class().is(&vm.ctx.types.tuple_type) {
                other
            } else {
                let elements = zelf
                    .as_slice()
                    .iter()
                    .chain(other.as_slice())
                    .cloned()
                    .collect::<Box<[_]>>();
                Self { elements }.into_ref(vm)
            }
        });
        PyArithmeticValue::from_option(added.ok())
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.elements.is_empty()
    }

    #[pymethod]
    fn count(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self.elements.iter() {
            if vm.identical_or_equal(element, &needle)? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[pymethod(magic)]
    #[inline]
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let mut str_parts = Vec::with_capacity(zelf.elements.len());
            for elem in zelf.elements.iter() {
                let s = elem.repr(vm)?;
                str_parts.push(s.as_str().to_owned());
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

    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Ok(if zelf.elements.is_empty() || value == 0 {
            vm.ctx.empty_tuple.clone()
        } else if value == 1 && zelf.class().is(&vm.ctx.types.tuple_type) {
            // Special case: when some `tuple` is multiplied by `1`,
            // nothing really happens, we need to return an object itself
            // with the same `id()` to be compatible with CPython.
            // This only works for `tuple` itself, not its subclasses.
            zelf
        } else {
            let v = zelf.elements.mul(vm, value)?;
            let elements = v.into_boxed_slice();
            Self { elements }.into_ref(vm)
        })
    }

    #[pymethod(magic)]
    fn getitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let result = match zelf.elements.as_ref().get_item(vm, needle, Self::NAME)? {
            Either::A(obj) => obj,
            Either::B(vec) => vm.ctx.new_tuple(vec).into(),
        };
        Ok(result)
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
            start += self.as_slice().len() as isize;
            if start < 0 {
                start = 0;
            }
        }
        let mut stop = stop.into_option().unwrap_or(sys::MAXSIZE);
        if stop < 0 {
            stop += self.as_slice().len() as isize;
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

    #[pymethod(magic)]
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
            PyTuple::new_ref(zelf.elements.clone().into_vec(), &vm.ctx)
        };
        (tup_arg,)
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

impl AsMapping for PyTuple {
    fn as_mapping(_zelf: &crate::PyObjectView<Self>, _vm: &VirtualMachine) -> PyMappingMethods {
        PyMappingMethods {
            length: Some(Self::length),
            subscript: Some(Self::subscript),
            ass_subscript: None,
        }
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
        _needle: PyObjectRef,
        _value: Option<PyObjectRef>,
        _vm: &VirtualMachine,
    ) -> PyResult<()> {
        unreachable!("ass_subscript not implemented for {}", zelf.class())
    }
}

impl Hashable for PyTuple {
    #[inline]
    fn hash(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        crate::utils::hash_iter(zelf.elements.iter(), vm)
    }
}

impl Comparable for PyTuple {
    fn cmp(
        zelf: &crate::PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(zelf, other) {
            return Ok(res.into());
        }
        let other = class_or_notimplemented!(Self, other);
        let a = zelf.as_slice();
        let b = other.as_slice();
        a.cmp(vm, b, op).map(PyComparisonValue::Implemented)
    }
}

impl Iterable for PyTuple {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyTupleIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
        }
        .into_object(vm))
    }
}

#[pyclass(module = false, name = "tuple_iterator")]
#[derive(Debug)]
pub(crate) struct PyTupleIterator {
    internal: PyMutex<PositionIterInternal<PyTupleRef>>,
}

impl PyValue for PyTupleIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.tuple_iterator_type
    }
}

#[pyimpl(with(Constructor, IterNext))]
impl PyTupleIterator {
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
    fn reduce(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.internal
            .lock()
            .builtins_iter_reduce(|x| x.clone().into(), vm)
    }
}
impl Unconstructible for PyTupleIterator {}

impl IterNextIterable for PyTupleIterator {}
impl IterNext for PyTupleIterator {
    fn next(zelf: &crate::PyObjectView<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|tuple, pos| {
            Ok(PyIterReturn::from_result(
                tuple.as_slice().get(pos).cloned().ok_or(None),
            ))
        })
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
        for elem in tuple.as_slice() {
            T::check(vm, elem)?
        }
        // SAFETY: the contract of TransmuteFromObject upholds the variant on `tuple`
        Ok(Self {
            tuple,
            _marker: PhantomData,
        })
    }
}

impl<T: TransmuteFromObject> PyTupleTyped<T> {
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        unsafe { &*(self.tuple.as_slice() as *const [PyObjectRef] as *const [T]) }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.tuple.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tuple.is_empty()
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

impl<T: TransmuteFromObject + fmt::Debug> fmt::Debug for PyTupleTyped<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_slice().fmt(f)
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
        self.tuple.into()
    }
}
