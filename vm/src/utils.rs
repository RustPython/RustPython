use crate::{
    builtins::{PyFloat, PyStr},
    convert::{ToPyException, ToPyObject},
    AsObject, PyObject, PyObjectRef, PyObjectWrap, PyResult, TryFromObject, VirtualMachine,
};
use num_traits::ToPrimitive;
use std::borrow::Borrow;

pub enum Either<A, B> {
    A(A),
    B(B),
}

impl<A: Borrow<PyObject>, B: Borrow<PyObject>> Borrow<PyObject> for Either<A, B> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        match self {
            Either::A(a) => a.borrow(),
            Either::B(b) => b.borrow(),
        }
    }
}

impl<A: AsRef<PyObject>, B: AsRef<PyObject>> AsRef<PyObject> for Either<A, B> {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        match self {
            Either::A(a) => a.as_ref(),
            Either::B(b) => b.as_ref(),
        }
    }
}

impl<A: PyObjectWrap, B: PyObjectWrap> PyObjectWrap for Either<A, B> {
    #[inline(always)]
    fn into_object(self) -> PyObjectRef {
        match self {
            Either::A(a) => a.into_object(),
            Either::B(b) => b.into_object(),
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
    vm.state.hash_secret.hash_iter(iter, |obj| obj.hash(vm))
}

pub fn hash_iter_unordered<'a, I: IntoIterator<Item = &'a PyObjectRef>>(
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<rustpython_common::hash::PyHash> {
    rustpython_common::hash::hash_iter_unordered(iter, |obj| obj.hash(vm))
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

impl ToPyObject for std::convert::Infallible {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        match self {}
    }
}

pub trait ToCString {
    fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<std::ffi::CString>;
}

impl ToCString for &str {
    fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<std::ffi::CString> {
        std::ffi::CString::new(*self).map_err(|err| err.to_pyexception(vm))
    }
}

impl ToCString for PyStr {
    fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<std::ffi::CString> {
        std::ffi::CString::new(self.as_ref()).map_err(|err| err.to_pyexception(vm))
    }
}

pub(crate) fn collection_repr<'a, I>(
    class_name: Option<&str>,
    prefix: &str,
    suffix: &str,
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<String>
where
    I: std::iter::Iterator<Item = &'a PyObjectRef>,
{
    let mut repr = String::new();
    if let Some(name) = class_name {
        repr.push_str(name);
        repr.push('(');
    }
    repr.push_str(prefix);
    {
        let mut parts_iter = iter.map(|o| o.repr(vm));
        repr.push_str(
            parts_iter
                .next()
                .transpose()?
                .expect("this is not called for empty collection")
                .as_str(),
        );
        for part in parts_iter {
            repr.push_str(", ");
            repr.push_str(part?.as_str());
        }
    }
    repr.push_str(suffix);
    if class_name.is_some() {
        repr.push(')');
    }

    Ok(repr)
}
