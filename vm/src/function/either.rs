use crate::{
    convert::ToPyObject, AsObject, PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
};
use std::borrow::Borrow;

pub enum Either<A, B> {
    A(A),
    B(B),
}

impl<A: Borrow<PyObject>, B: Borrow<PyObject>> Borrow<PyObject> for Either<A, B> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        match self {
            Self::A(a) => a.borrow(),
            Self::B(b) => b.borrow(),
        }
    }
}

impl<A: AsRef<PyObject>, B: AsRef<PyObject>> AsRef<PyObject> for Either<A, B> {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        match self {
            Self::A(a) => a.as_ref(),
            Self::B(b) => b.as_ref(),
        }
    }
}

impl<A: Into<PyObjectRef>, B: Into<PyObjectRef>> From<Either<A, B>> for PyObjectRef {
    #[inline(always)]
    fn from(value: Either<A, B>) -> Self {
        match value {
            Either::A(a) => a.into(),
            Either::B(b) => b.into(),
        }
    }
}

impl<A: ToPyObject, B: ToPyObject> ToPyObject for Either<A, B> {
    #[inline(always)]
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::A(a) => a.to_pyobject(vm),
            Self::B(b) => b.to_pyobject(vm),
        }
    }
}

/// This allows a builtin method to accept arguments that may be one of two
/// types, raising a `TypeError` if it is neither.
///
/// # Example
///
/// ```
/// use rustpython_vm::VirtualMachine;
/// use rustpython_vm::builtins::{PyStrRef, PyIntRef};
/// use rustpython_vm::function::Either;
///
/// fn do_something(arg: Either<PyIntRef, PyStrRef>, vm: &VirtualMachine) {
///     match arg {
///         Either::A(int)=> {
///             // do something with int
///         }
///         Either::B(string) => {
///             // do something with string
///         }
///     }
/// }
/// ```
impl<A, B> TryFromObject for Either<A, B>
where
    A: TryFromObject,
    B: TryFromObject,
{
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        A::try_from_object(vm, obj.clone())
            .map(Either::A)
            .or_else(|_| B::try_from_object(vm, obj.clone()).map(Either::B))
            .map_err(|_| vm.new_type_error(format!("unexpected type {}", obj.class())))
    }
}
