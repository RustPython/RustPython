use crate::{
    builtins::iter::PySequenceIterator,
    convert::{ToPyObject, ToPyResult},
    object::{Traverse, TraverseFn},
    AsObject, PyObject, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
};
use std::borrow::Borrow;
use std::ops::Deref;

/// Iterator Protocol
// https://docs.python.org/3/c-api/iter.html
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct PyIter<O = PyObjectRef>(O)
where
    O: Borrow<PyObject>;

unsafe impl<O: Borrow<PyObject>> Traverse for PyIter<O> {
    fn traverse(&self, tracer_fn: &mut TraverseFn) {
        self.0.borrow().traverse(tracer_fn);
    }
}

impl PyIter<PyObjectRef> {
    pub fn check(obj: &PyObject) -> bool {
        obj.class()
            .mro_find_map(|x| x.slots.iternext.load())
            .is_some()
    }
}

impl<O> PyIter<O>
where
    O: Borrow<PyObject>,
{
    pub fn new(obj: O) -> Self {
        Self(obj)
    }
    pub fn next(&self, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let iternext = {
            self.0
                .borrow()
                .class()
                .mro_find_map(|x| x.slots.iternext.load())
                .ok_or_else(|| {
                    vm.new_type_error(format!(
                        "'{}' object is not an iterator",
                        self.0.borrow().class().name()
                    ))
                })?
        };
        iternext(self.0.borrow(), vm)
    }

    pub fn iter<'a, 'b, U>(
        &'b self,
        vm: &'a VirtualMachine,
    ) -> PyResult<PyIterIter<'a, U, &'b PyObject>> {
        let length_hint = vm.length_hint_opt(self.as_ref().to_owned())?;
        Ok(PyIterIter::new(vm, self.0.borrow(), length_hint))
    }

    pub fn iter_without_hint<'a, 'b, U>(
        &'b self,
        vm: &'a VirtualMachine,
    ) -> PyResult<PyIterIter<'a, U, &'b PyObject>> {
        Ok(PyIterIter::new(vm, self.0.borrow(), None))
    }
}

impl PyIter<PyObjectRef> {
    /// Returns an iterator over this sequence of objects.
    pub fn into_iter<U>(self, vm: &VirtualMachine) -> PyResult<PyIterIter<U, PyObjectRef>> {
        let length_hint = vm.length_hint_opt(self.as_object().to_owned())?;
        Ok(PyIterIter::new(vm, self.0, length_hint))
    }
}

impl From<PyIter<PyObjectRef>> for PyObjectRef {
    fn from(value: PyIter<PyObjectRef>) -> PyObjectRef {
        value.0
    }
}

impl<O> Borrow<PyObject> for PyIter<O>
where
    O: Borrow<PyObject>,
{
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self.0.borrow()
    }
}

impl<O> AsRef<PyObject> for PyIter<O>
where
    O: Borrow<PyObject>,
{
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.0.borrow()
    }
}

impl<O> Deref for PyIter<O>
where
    O: Borrow<PyObject>,
{
    type Target = PyObject;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.0.borrow()
    }
}

impl ToPyObject for PyIter<PyObjectRef> {
    #[inline(always)]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.into()
    }
}

impl TryFromObject for PyIter<PyObjectRef> {
    // This helper function is called at multiple places. First, it is called
    // in the vm when a for loop is entered. Next, it is used when the builtin
    // function 'iter' is called.
    fn try_from_object(vm: &VirtualMachine, iter_target: PyObjectRef) -> PyResult<Self> {
        let getiter = {
            let cls = iter_target.class();
            cls.mro_find_map(|x| x.slots.iter.load())
        };
        if let Some(getiter) = getiter {
            let iter = getiter(iter_target, vm)?;
            if PyIter::check(&iter) {
                Ok(Self(iter))
            } else {
                Err(vm.new_type_error(format!(
                    "iter() returned non-iterator of type '{}'",
                    iter.class().name()
                )))
            }
        } else if let Ok(seq_iter) = PySequenceIterator::new(iter_target.clone(), vm) {
            Ok(Self(seq_iter.into_pyobject(vm)))
        } else {
            Err(vm.new_type_error(format!(
                "'{}' object is not iterable",
                iter_target.class().name()
            )))
        }
    }
}

#[derive(result_like::ResultLike)]
pub enum PyIterReturn<T = PyObjectRef> {
    Return(T),
    StopIteration(Option<PyObjectRef>),
}

unsafe impl<T: Traverse> Traverse for PyIterReturn<T> {
    fn traverse(&self, tracer_fn: &mut TraverseFn) {
        match self {
            PyIterReturn::Return(r) => r.traverse(tracer_fn),
            PyIterReturn::StopIteration(Some(obj)) => obj.traverse(tracer_fn),
            _ => (),
        }
    }
}

impl PyIterReturn {
    pub fn from_pyresult(result: PyResult, vm: &VirtualMachine) -> PyResult<Self> {
        match result {
            Ok(obj) => Ok(Self::Return(obj)),
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.stop_iteration) => {
                let args = err.get_arg(0);
                Ok(Self::StopIteration(args))
            }
            Err(err) => Err(err),
        }
    }

    pub fn from_getitem_result(result: PyResult, vm: &VirtualMachine) -> PyResult<Self> {
        match result {
            Ok(obj) => Ok(Self::Return(obj)),
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.index_error) => {
                Ok(Self::StopIteration(None))
            }
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.stop_iteration) => {
                let args = err.get_arg(0);
                Ok(Self::StopIteration(args))
            }
            Err(err) => Err(err),
        }
    }

    pub fn into_async_pyresult(self, vm: &VirtualMachine) -> PyResult {
        match self {
            Self::Return(obj) => Ok(obj),
            Self::StopIteration(v) => Err({
                let args = if let Some(v) = v { vec![v] } else { Vec::new() };
                vm.new_exception(vm.ctx.exceptions.stop_async_iteration.to_owned(), args)
            }),
        }
    }
}

impl ToPyResult for PyIterReturn {
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult {
        match self {
            Self::Return(obj) => Ok(obj),
            Self::StopIteration(v) => Err(vm.new_stop_iteration(v)),
        }
    }
}

impl ToPyResult for PyResult<PyIterReturn> {
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult {
        self?.to_pyresult(vm)
    }
}

// Typical rust `Iter` object for `PyIter`
pub struct PyIterIter<'a, T, O = PyObjectRef>
where
    O: Borrow<PyObject>,
{
    vm: &'a VirtualMachine,
    obj: O, // creating PyIter<O> is zero-cost
    length_hint: Option<usize>,
    _phantom: std::marker::PhantomData<T>,
}

unsafe impl<'a, T, O> Traverse for PyIterIter<'a, T, O>
where
    O: Traverse + Borrow<PyObject>,
{
    fn traverse(&self, tracer_fn: &mut TraverseFn) {
        self.obj.traverse(tracer_fn)
    }
}

impl<'a, T, O> PyIterIter<'a, T, O>
where
    O: Borrow<PyObject>,
{
    pub fn new(vm: &'a VirtualMachine, obj: O, length_hint: Option<usize>) -> Self {
        Self {
            vm,
            obj,
            length_hint,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<'a, T, O> Iterator for PyIterIter<'a, T, O>
where
    T: TryFromObject,
    O: Borrow<PyObject>,
{
    type Item = PyResult<T>;

    fn next(&mut self) -> Option<Self::Item> {
        let imp = |next: PyResult<PyIterReturn>| -> PyResult<Option<T>> {
            let Some(obj) = next?.into_result().ok() else {
                return Ok(None);
            };
            Ok(Some(T::try_from_object(self.vm, obj)?))
        };
        let next = PyIter::new(self.obj.borrow()).next(self.vm);
        imp(next).transpose()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.length_hint.unwrap_or(0), self.length_hint)
    }
}
