use super::IntoFuncArgs;
use crate::{
    builtins::iter::PySequenceIterator, protocol::PyIter, protocol::PyIterIter, PyObject,
    PyObjectRef, PyObjectWrap, PyResult, PyValue, TryFromObject, TypeProtocol, VirtualMachine,
};
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub struct ArgCallable {
    obj: PyObjectRef,
}

impl ArgCallable {
    #[inline]
    pub fn invoke(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        vm.invoke(&self.obj, args)
    }
}

impl AsRef<PyObject> for ArgCallable {
    fn as_ref(&self) -> &PyObject {
        &self.obj
    }
}

impl PyObjectWrap for ArgCallable {
    fn into_object(self) -> PyObjectRef {
        self.obj
    }
}

impl TryFromObject for ArgCallable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if vm.is_callable(&obj) {
            Ok(ArgCallable { obj })
        } else {
            Err(vm.new_type_error(format!("'{}' object is not callable", obj.class().name())))
        }
    }
}

/// An iterable Python object.
///
/// `ArgIterable` implements `FromArgs` so that a built-in function can accept
/// an object that is required to conform to the Python iterator protocol.
///
/// ArgIterable can optionally perform type checking and conversions on iterated
/// objects using a generic type parameter that implements `TryFromObject`.
pub struct ArgIterable<T = PyObjectRef> {
    iterable: PyObjectRef,
    iterfn: Option<crate::types::IterFunc>,
    _item: PhantomData<T>,
}

impl<T> ArgIterable<T> {
    /// Returns an iterator over this sequence of objects.
    ///
    /// This operation may fail if an exception is raised while invoking the
    /// `__iter__` method of the iterable object.
    pub fn iter<'a>(&self, vm: &'a VirtualMachine) -> PyResult<PyIterIter<'a, T>> {
        let iter = PyIter::new(match self.iterfn {
            Some(f) => f(self.iterable.clone(), vm)?,
            None => PySequenceIterator::new(self.iterable.clone()).into_object(vm),
        });
        iter.into_iter(vm)
    }
}

impl<T> TryFromObject for ArgIterable<T>
where
    T: TryFromObject,
{
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let iterfn;
        {
            let cls = obj.class();
            iterfn = cls.mro_find_map(|x| x.slots.iter.load());
            if iterfn.is_none() && !cls.has_attr("__getitem__") {
                return Err(vm.new_type_error(format!("'{}' object is not iterable", cls.name())));
            }
        }
        Ok(Self {
            iterable: obj,
            iterfn,
            _item: PhantomData,
        })
    }
}
