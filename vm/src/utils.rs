use crate::{
    builtins::{PyFloat, PyStr},
    function::{IntoPyException, IntoPyObject},
    PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol, VirtualMachine,
};
use num_traits::ToPrimitive;

pub enum Either<A, B> {
    A(A),
    B(B),
}

impl<A: PyValue, B: PyValue> Either<PyRef<A>, PyRef<B>> {
    pub fn as_object(&self) -> &PyObjectRef {
        match self {
            Either::A(a) => a.as_object(),
            Either::B(b) => b.as_object(),
        }
    }

    pub fn into_object(self) -> PyObjectRef {
        match self {
            Either::A(a) => a.into_object(),
            Either::B(b) => b.into_object(),
        }
    }
}

impl<A: IntoPyObject, B: IntoPyObject> IntoPyObject for Either<A, B> {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::A(a) => a.into_pyobject(vm),
            Self::B(b) => b.into_pyobject(vm),
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
/// use rustpython_vm::utils::Either;
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

pub fn hash_iter<'a, I: IntoIterator<Item = &'a PyObjectRef>>(
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<rustpython_common::hash::PyHash> {
    vm.state.hash_secret.hash_iter(iter, |obj| vm._hash(obj))
}

pub fn hash_iter_unordered<'a, I: IntoIterator<Item = &'a PyObjectRef>>(
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<rustpython_common::hash::PyHash> {
    rustpython_common::hash::hash_iter_unordered(iter, |obj| vm._hash(obj))
}

// TODO: find a better place to put this impl
impl TryFromObject for std::time::Duration {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        use std::time::Duration;
        if let Some(float) = obj.payload::<PyFloat>() {
            Ok(Duration::from_secs_f64(float.to_f64()))
        } else if let Some(int) = vm.to_index_opt(obj.clone()) {
            let sec = int?
                .as_bigint()
                .to_u64()
                .ok_or_else(|| vm.new_value_error("value out of range".to_owned()))?;
            Ok(Duration::from_secs(sec))
        } else {
            Err(vm.new_type_error(format!(
                "expected an int or float for duration, got {}",
                obj.class()
            )))
        }
    }
}

impl IntoPyObject for std::convert::Infallible {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        match self {}
    }
}

pub trait ToCString {
    fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<std::ffi::CString>;
}

impl ToCString for &str {
    fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<std::ffi::CString> {
        std::ffi::CString::new(*self).map_err(|err| err.into_pyexception(vm))
    }
}

impl ToCString for PyStr {
    fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<std::ffi::CString> {
        std::ffi::CString::new(self.as_ref()).map_err(|err| err.into_pyexception(vm))
    }
}
