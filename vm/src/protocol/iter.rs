use crate::{
    builtins::iter::PySequenceIterator,
    function::{IntoPyObject, IntoPyResult},
    PyObjectRef, PyObjectWrap, PyResult, PyValue, TryFromObject, TypeProtocol, VirtualMachine,
};
use std::borrow::Borrow;
use std::ops::Deref;

/// Iterator Protocol
// https://docs.python.org/3/c-api/iter.html
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct PyIter<O = PyObjectRef>(O)
where
    O: Borrow<PyObjectRef>;

impl PyIter<PyObjectRef> {
    pub fn check(obj: &PyObjectRef) -> bool {
        obj.class()
            .mro_find_map(|x| x.slots.iternext.load())
            .is_some()
    }
}

impl<O> PyIter<O>
where
    O: Borrow<PyObjectRef>,
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
    ) -> PyResult<PyIterIter<'a, U, &'b PyObjectRef>> {
        let length_hint = vm.length_hint(self.as_ref().clone())?;
        Ok(PyIterIter::new(vm, self.0.borrow(), length_hint))
    }

    pub fn iter_without_hint<'a, 'b, U>(
        &'b self,
        vm: &'a VirtualMachine,
    ) -> PyResult<PyIterIter<'a, U, &'b PyObjectRef>> {
        Ok(PyIterIter::new(vm, self.0.borrow(), None))
    }
}

impl PyIter<PyObjectRef> {
    /// Returns an iterator over this sequence of objects.
    pub fn into_iter<U>(self, vm: &VirtualMachine) -> PyResult<PyIterIter<U, PyObjectRef>> {
        let length_hint = vm.length_hint(self.as_object().clone())?;
        Ok(PyIterIter::new(vm, self.0, length_hint))
    }
}

impl PyObjectWrap for PyIter<PyObjectRef> {
    fn into_object(self) -> PyObjectRef {
        self.0
    }
}

impl<O> AsRef<PyObjectRef> for PyIter<O>
where
    O: Borrow<PyObjectRef>,
{
    fn as_ref(&self) -> &PyObjectRef {
        self.0.borrow()
    }
}

impl<O> Deref for PyIter<O>
where
    O: Borrow<PyObjectRef>,
{
    type Target = PyObjectRef;
    fn deref(&self) -> &Self::Target {
        self.0.borrow()
    }
}

impl IntoPyObject for PyIter<PyObjectRef> {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
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
        } else {
            // TODO: __getitem__ method lookup must be replaced by sequence protocol checking
            vm.get_method_or_type_error(iter_target.clone(), "__getitem__", || {
                format!("'{}' object is not iterable", iter_target.class().name())
            })?;
            Ok(Self(PySequenceIterator::new(iter_target).into_object(vm)))
        }
    }
}

impl PyObjectRef {
    /// Takes an object and returns an iterator for it.
    /// This is typically a new iterator but if the argument is an iterator, this
    /// returns itself.
    pub fn get_iter(self, vm: &VirtualMachine) -> PyResult<PyIter> {
        // PyObject_GetIter
        PyIter::try_from_object(vm, self)
    }
}

pub enum PyIterReturn<T = PyObjectRef> {
    Return(T),
    StopIteration(Option<PyObjectRef>),
}

impl PyIterReturn {
    pub fn from_pyresult(result: PyResult, vm: &VirtualMachine) -> PyResult<Self> {
        match result {
            Ok(obj) => Ok(Self::Return(obj)),
            Err(err) if err.isinstance(&vm.ctx.exceptions.stop_iteration) => {
                let args = err.get_arg(0);
                Ok(Self::StopIteration(args))
            }
            Err(err) => Err(err),
        }
    }

    pub fn from_getitem_result(result: PyResult, vm: &VirtualMachine) -> PyResult<Self> {
        match result {
            Ok(obj) => Ok(Self::Return(obj)),
            Err(err) if err.isinstance(&vm.ctx.exceptions.index_error) => {
                Ok(Self::StopIteration(None))
            }
            Err(err) if err.isinstance(&vm.ctx.exceptions.stop_iteration) => {
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
                vm.new_exception(vm.ctx.exceptions.stop_async_iteration.clone(), args)
            }),
        }
    }
}

impl IntoPyResult for PyIterReturn {
    fn into_pyresult(self, vm: &VirtualMachine) -> PyResult {
        match self {
            Self::Return(obj) => Ok(obj),
            Self::StopIteration(v) => Err(vm.new_stop_iteration(v)),
        }
    }
}

impl IntoPyResult for PyResult<PyIterReturn> {
    fn into_pyresult(self, vm: &VirtualMachine) -> PyResult {
        self.and_then(|obj| obj.into_pyresult(vm))
    }
}

// Typical rust `Iter` object for `PyIter`
pub struct PyIterIter<'a, T, O = PyObjectRef>
where
    O: Borrow<PyObjectRef>,
{
    vm: &'a VirtualMachine,
    obj: O, // creating PyIter<O> is zero-cost
    length_hint: Option<usize>,
    _phantom: std::marker::PhantomData<T>,
}

impl<'a, T, O> PyIterIter<'a, T, O>
where
    O: Borrow<PyObjectRef>,
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
    O: Borrow<PyObjectRef>,
{
    type Item = PyResult<T>;

    fn next(&mut self) -> Option<Self::Item> {
        PyIter::new(self.obj.borrow())
            .next(self.vm)
            .map(|iret| match iret {
                PyIterReturn::Return(obj) => Some(obj),
                PyIterReturn::StopIteration(_) => None,
            })
            .transpose()
            .map(|x| x.and_then(|obj| T::try_from_object(self.vm, obj)))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.length_hint.unwrap_or(0), self.length_hint)
    }
}
