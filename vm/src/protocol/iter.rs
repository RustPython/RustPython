use crate::IntoPyObject;
use crate::{
    builtins::iter::PySequenceIterator, PyObjectRef, PyResult, PyValue, TryFromObject,
    TypeProtocol, VirtualMachine,
};
use std::borrow::Borrow;
use std::ops::Deref;

/// Iterator Protocol
// https://docs.python.org/3/c-api/iter.html
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct PyIter<T = PyObjectRef>(T)
where
    T: Borrow<PyObjectRef>;

impl PyIter<PyObjectRef> {
    pub fn into_object(self) -> PyObjectRef {
        self.0
    }
}

impl<T> PyIter<T>
where
    T: Borrow<PyObjectRef>,
{
    pub fn new(obj: T) -> Self {
        Self(obj)
    }
    pub fn as_object(&self) -> &PyObjectRef {
        self.0.borrow()
    }
    pub fn next(&self, vm: &VirtualMachine) -> PyResult {
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
}

impl<T> Borrow<PyObjectRef> for PyIter<T>
where
    T: Borrow<PyObjectRef>,
{
    fn borrow(&self) -> &PyObjectRef {
        self.0.borrow()
    }
}

impl<T> Deref for PyIter<T>
where
    T: Borrow<PyObjectRef>,
{
    type Target = PyObjectRef;
    fn deref(&self) -> &Self::Target {
        self.0.borrow()
    }
}

impl IntoPyObject for PyIter<PyObjectRef> {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.into_object()
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
            let cls = iter.class();
            let is_iter = cls.iter_mro().any(|x| x.slots.iternext.load().is_some());
            if is_iter {
                drop(cls);
                Ok(Self(iter))
            } else {
                Err(vm.new_type_error(format!(
                    "iter() returned non-iterator of type '{}'",
                    cls.name()
                )))
            }
        } else {
            vm.get_method_or_type_error(iter_target.clone(), "__getitem__", || {
                format!("'{}' object is not iterable", iter_target.class().name())
            })?;
            Ok(Self(
                PySequenceIterator::new(iter_target)
                    .into_ref(vm)
                    .into_object(),
            ))
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
