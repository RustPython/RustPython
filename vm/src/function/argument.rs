use super::IntoFuncArgs;
use crate::{
    builtins::iter::PySequenceIterator,
    iterator,
    protocol::{PyIter, PyIterReturn},
    PyObjectRef, PyResult, PyValue, TryFromObject, TypeProtocol, VirtualMachine,
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

    #[inline]
    pub fn into_object(self) -> PyObjectRef {
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
    iterfn: Option<crate::slots::IterFunc>,
    _item: PhantomData<T>,
}

impl<T> ArgIterable<T> {
    /// Returns an iterator over this sequence of objects.
    ///
    /// This operation may fail if an exception is raised while invoking the
    /// `__iter__` method of the iterable object.
    pub fn iter<'a>(&self, vm: &'a VirtualMachine) -> PyResult<PyIterator<'a, T>> {
        let iter_obj = match self.iterfn {
            Some(f) => f(self.iterable.clone(), vm)?,
            None => PySequenceIterator::new(self.iterable.clone()).into_object(vm),
        };

        let length_hint = iterator::length_hint(vm, iter_obj.clone())?;

        Ok(PyIterator {
            vm,
            obj: iter_obj,
            length_hint,
            _item: PhantomData,
        })
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
        Ok(ArgIterable {
            iterable: obj,
            iterfn,
            _item: PhantomData,
        })
    }
}

pub struct PyIterator<'a, T> {
    vm: &'a VirtualMachine,
    obj: PyObjectRef,
    length_hint: Option<usize>,
    _item: PhantomData<T>,
}

impl<'a, T> Iterator for PyIterator<'a, T>
where
    T: TryFromObject,
{
    type Item = PyResult<T>;

    fn next(&mut self) -> Option<Self::Item> {
        PyIter::new(&self.obj)
            .next(self.vm)
            .map(|iret| match iret {
                PyIterReturn::Return(obj) => Some(obj),
                PyIterReturn::StopIteration(_) => None,
            })
            .transpose()
            .map(|x| x.and_then(|obj| T::try_from_object(self.vm, obj)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.length_hint.unwrap_or(0), self.length_hint)
    }
}
