/*! Python `attribute` descriptor class. (PyGetSet)

*/
use crate::{
    convert::ToPyResult,
    function::{OwnedParam, RefParam},
    object::PyThreadingConstraint,
    PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
};

#[derive(result_like::OptionLike, is_macro::Is)]
pub enum PySetterValue<T = PyObjectRef> {
    Assign(T),
    Delete,
}

impl PySetterValue {
    pub fn unwrap_or_none(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::Assign(value) => value,
            Self::Delete => vm.ctx.none(),
        }
    }
}

trait FromPySetterValue
where
    Self: Sized,
{
    fn from_setter_value(vm: &VirtualMachine, obj: PySetterValue) -> PyResult<Self>;
}

impl<T> FromPySetterValue for T
where
    T: Sized + TryFromObject,
{
    #[inline]
    fn from_setter_value(vm: &VirtualMachine, obj: PySetterValue) -> PyResult<Self> {
        let obj = obj.ok_or_else(|| vm.new_type_error("can't delete attribute".to_owned()))?;
        T::try_from_object(vm, obj)
    }
}

impl<T> FromPySetterValue for PySetterValue<T>
where
    T: Sized + TryFromObject,
{
    #[inline]
    fn from_setter_value(vm: &VirtualMachine, obj: PySetterValue) -> PyResult<Self> {
        obj.map(|obj| T::try_from_object(vm, obj)).transpose()
    }
}

pub type PyGetterFunc = Box<py_dyn_fn!(dyn Fn(&VirtualMachine, PyObjectRef) -> PyResult)>;
pub type PySetterFunc =
    Box<py_dyn_fn!(dyn Fn(&VirtualMachine, PyObjectRef, PySetterValue) -> PyResult<()>)>;

pub trait IntoPyGetterFunc<T>: PyThreadingConstraint + Sized + 'static {
    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult;
    fn into_getter(self) -> PyGetterFunc {
        Box::new(move |vm, obj| self.get(obj, vm))
    }
}

impl<F, T, R> IntoPyGetterFunc<(OwnedParam<T>, R, VirtualMachine)> for F
where
    F: Fn(T, &VirtualMachine) -> R + 'static + Send + Sync,
    T: TryFromObject,
    R: ToPyResult,
{
    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let obj = T::try_from_object(vm, obj)?;
        (self)(obj, vm).to_pyresult(vm)
    }
}

impl<F, S, R> IntoPyGetterFunc<(RefParam<S>, R, VirtualMachine)> for F
where
    F: Fn(&S, &VirtualMachine) -> R + 'static + Send + Sync,
    S: PyPayload,
    R: ToPyResult,
{
    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let zelf = PyRef::<S>::try_from_object(vm, obj)?;
        (self)(&zelf, vm).to_pyresult(vm)
    }
}

impl<F, T, R> IntoPyGetterFunc<(OwnedParam<T>, R)> for F
where
    F: Fn(T) -> R + 'static + Send + Sync,
    T: TryFromObject,
    R: ToPyResult,
{
    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let obj = T::try_from_object(vm, obj)?;
        (self)(obj).to_pyresult(vm)
    }
}

impl<F, S, R> IntoPyGetterFunc<(RefParam<S>, R)> for F
where
    F: Fn(&S) -> R + 'static + Send + Sync,
    S: PyPayload,
    R: ToPyResult,
{
    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let zelf = PyRef::<S>::try_from_object(vm, obj)?;
        (self)(&zelf).to_pyresult(vm)
    }
}

pub trait IntoPyNoResult {
    fn into_noresult(self) -> PyResult<()>;
}

impl IntoPyNoResult for () {
    #[inline]
    fn into_noresult(self) -> PyResult<()> {
        Ok(())
    }
}

impl IntoPyNoResult for PyResult<()> {
    #[inline]
    fn into_noresult(self) -> PyResult<()> {
        self
    }
}

pub trait IntoPySetterFunc<T>: PyThreadingConstraint + Sized + 'static {
    fn set(&self, obj: PyObjectRef, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()>;
    fn into_setter(self) -> PySetterFunc {
        Box::new(move |vm, obj, value| self.set(obj, value, vm))
    }
}

impl<F, T, V, R> IntoPySetterFunc<(OwnedParam<T>, V, R, VirtualMachine)> for F
where
    F: Fn(T, V, &VirtualMachine) -> R + 'static + Send + Sync,
    T: TryFromObject,
    V: FromPySetterValue,
    R: IntoPyNoResult,
{
    fn set(&self, obj: PyObjectRef, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        let obj = T::try_from_object(vm, obj)?;
        let value = V::from_setter_value(vm, value)?;
        (self)(obj, value, vm).into_noresult()
    }
}

impl<F, S, V, R> IntoPySetterFunc<(RefParam<S>, V, R, VirtualMachine)> for F
where
    F: Fn(&S, V, &VirtualMachine) -> R + 'static + Send + Sync,
    S: PyPayload,
    V: FromPySetterValue,
    R: IntoPyNoResult,
{
    fn set(&self, obj: PyObjectRef, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        let zelf = PyRef::<S>::try_from_object(vm, obj)?;
        let value = V::from_setter_value(vm, value)?;
        (self)(&zelf, value, vm).into_noresult()
    }
}

impl<F, T, V, R> IntoPySetterFunc<(OwnedParam<T>, V, R)> for F
where
    F: Fn(T, V) -> R + 'static + Send + Sync,
    T: TryFromObject,
    V: FromPySetterValue,
    R: IntoPyNoResult,
{
    fn set(&self, obj: PyObjectRef, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        let obj = T::try_from_object(vm, obj)?;
        let value = V::from_setter_value(vm, value)?;
        (self)(obj, value).into_noresult()
    }
}

impl<F, S, V, R> IntoPySetterFunc<(RefParam<S>, V, R)> for F
where
    F: Fn(&S, V) -> R + 'static + Send + Sync,
    S: PyPayload,
    V: FromPySetterValue,
    R: IntoPyNoResult,
{
    fn set(&self, obj: PyObjectRef, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        let zelf = PyRef::<S>::try_from_object(vm, obj)?;
        let value = V::from_setter_value(vm, value)?;
        (self)(&zelf, value).into_noresult()
    }
}
