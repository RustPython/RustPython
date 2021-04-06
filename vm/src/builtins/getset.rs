/*! Python `attribute` descriptor class. (PyGetSet)

*/
use super::pytype::PyTypeRef;
use crate::function::{OwnedParam, RefParam};
use crate::pyobject::{
    IntoPyResult, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyThreadingConstraint,
    PyValue, TryFromObject, TypeProtocol,
};
use crate::slots::SlotDescriptor;
use crate::vm::VirtualMachine;

pub type PyGetterFunc = Box<py_dyn_fn!(dyn Fn(&VirtualMachine, PyObjectRef) -> PyResult)>;
pub type PySetterFunc =
    Box<py_dyn_fn!(dyn Fn(&VirtualMachine, PyObjectRef, PyObjectRef) -> PyResult<()>)>;
pub type PyDeleterFunc = Box<py_dyn_fn!(dyn Fn(&VirtualMachine, PyObjectRef) -> PyResult<()>)>;

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
    R: IntoPyResult,
{
    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let obj = T::try_from_object(vm, obj)?;
        (self)(obj, vm).into_pyresult(vm)
    }
}

impl<F, S, R> IntoPyGetterFunc<(RefParam<S>, R, VirtualMachine)> for F
where
    F: Fn(&S, &VirtualMachine) -> R + 'static + Send + Sync,
    S: PyValue,
    R: IntoPyResult,
{
    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let zelf = PyRef::<S>::try_from_object(vm, obj)?;
        (self)(&zelf, vm).into_pyresult(vm)
    }
}

impl<F, T, R> IntoPyGetterFunc<(OwnedParam<T>, R)> for F
where
    F: Fn(T) -> R + 'static + Send + Sync,
    T: TryFromObject,
    R: IntoPyResult,
{
    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let obj = T::try_from_object(vm, obj)?;
        (self)(obj).into_pyresult(vm)
    }
}

impl<F, S, R> IntoPyGetterFunc<(RefParam<S>, R)> for F
where
    F: Fn(&S) -> R + 'static + Send + Sync,
    S: PyValue,
    R: IntoPyResult,
{
    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let zelf = PyRef::<S>::try_from_object(vm, obj)?;
        (self)(&zelf).into_pyresult(vm)
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
    fn set(&self, obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()>;
    fn into_setter(self) -> PySetterFunc {
        Box::new(move |vm, obj, value| self.set(obj, value, vm))
    }
}

impl<F, T, V, R> IntoPySetterFunc<(OwnedParam<T>, V, R, VirtualMachine)> for F
where
    F: Fn(T, V, &VirtualMachine) -> R + 'static + Send + Sync,
    T: TryFromObject,
    V: TryFromObject,
    R: IntoPyNoResult,
{
    fn set(&self, obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let obj = T::try_from_object(vm, obj)?;
        let value = V::try_from_object(vm, value)?;
        (self)(obj, value, vm).into_noresult()
    }
}

impl<F, S, V, R> IntoPySetterFunc<(RefParam<S>, V, R, VirtualMachine)> for F
where
    F: Fn(&S, V, &VirtualMachine) -> R + 'static + Send + Sync,
    S: PyValue,
    V: TryFromObject,
    R: IntoPyNoResult,
{
    fn set(&self, obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let zelf = PyRef::<S>::try_from_object(vm, obj)?;
        let value = V::try_from_object(vm, value)?;
        (self)(&zelf, value, vm).into_noresult()
    }
}

impl<F, T, V, R> IntoPySetterFunc<(OwnedParam<T>, V, R)> for F
where
    F: Fn(T, V) -> R + 'static + Send + Sync,
    T: TryFromObject,
    V: TryFromObject,
    R: IntoPyNoResult,
{
    fn set(&self, obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let obj = T::try_from_object(vm, obj)?;
        let value = V::try_from_object(vm, value)?;
        (self)(obj, value).into_noresult()
    }
}

impl<F, S, V, R> IntoPySetterFunc<(RefParam<S>, V, R)> for F
where
    F: Fn(&S, V) -> R + 'static + Send + Sync,
    S: PyValue,
    V: TryFromObject,
    R: IntoPyNoResult,
{
    fn set(&self, obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let zelf = PyRef::<S>::try_from_object(vm, obj)?;
        let value = V::try_from_object(vm, value)?;
        (self)(&zelf, value).into_noresult()
    }
}

pub trait IntoPyDeleterFunc<T>: PyThreadingConstraint + Sized + 'static {
    fn delete(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()>;
    fn into_deleter(self) -> PyDeleterFunc {
        Box::new(move |vm, obj| self.delete(obj, vm))
    }
}

impl<F, T, R> IntoPyDeleterFunc<(OwnedParam<T>, R, VirtualMachine)> for F
where
    F: Fn(T, &VirtualMachine) -> R + 'static + Send + Sync,
    T: TryFromObject,
    R: IntoPyNoResult,
{
    fn delete(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let obj = T::try_from_object(vm, obj)?;
        (self)(obj, vm).into_noresult()
    }
}

impl<F, S, R> IntoPyDeleterFunc<(RefParam<S>, R, VirtualMachine)> for F
where
    F: Fn(&S, &VirtualMachine) -> R + 'static + Send + Sync,
    S: PyValue,
    R: IntoPyNoResult,
{
    fn delete(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let zelf = PyRef::<S>::try_from_object(vm, obj)?;
        (self)(&zelf, vm).into_noresult()
    }
}

impl<F, T, R> IntoPyDeleterFunc<(OwnedParam<T>, R)> for F
where
    F: Fn(T) -> R + 'static + Send + Sync,
    T: TryFromObject,
    R: IntoPyNoResult,
{
    fn delete(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let obj = T::try_from_object(vm, obj)?;
        (self)(obj).into_noresult()
    }
}

impl<F, S, R> IntoPyDeleterFunc<(RefParam<S>, R)> for F
where
    F: Fn(&S) -> R + 'static + Send + Sync,
    S: PyValue,
    R: IntoPyNoResult,
{
    fn delete(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let zelf = PyRef::<S>::try_from_object(vm, obj)?;
        (self)(&zelf).into_noresult()
    }
}

#[pyclass(module = false, name = "getset_descriptor")]
pub struct PyGetSet {
    name: String,
    getter: Option<PyGetterFunc>,
    setter: Option<PySetterFunc>,
    deleter: Option<PyDeleterFunc>,
    // doc: Option<String>,
}

impl std::fmt::Debug for PyGetSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PyGetSet {{ name: {}, getter: {}, setter: {} }}",
            self.name,
            if self.getter.is_some() {
                "Some"
            } else {
                "None"
            },
            if self.setter.is_some() {
                "Some"
            } else {
                "None"
            },
        )
    }
}

impl PyValue for PyGetSet {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.getset_type
    }
}

impl SlotDescriptor for PyGetSet {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = match Self::_check(zelf, obj, vm) {
            Ok(obj) => obj,
            Err(result) => return result,
        };
        if let Some(ref f) = zelf.getter {
            f(vm, obj)
        } else {
            Err(vm.new_attribute_error(format!(
                "attribute '{}' of '{}' objects is not readable",
                zelf.name,
                Self::class(vm).name
            )))
        }
    }
}

impl PyGetSet {
    pub fn new(name: String) -> Self {
        Self {
            name,
            getter: None,
            setter: None,
            deleter: None,
        }
    }

    pub fn with_get<G, X>(mut self, getter: G) -> Self
    where
        G: IntoPyGetterFunc<X>,
    {
        self.getter = Some(getter.into_getter());
        self
    }

    pub fn with_set<S, X>(mut self, setter: S) -> Self
    where
        S: IntoPySetterFunc<X>,
    {
        self.setter = Some(setter.into_setter());
        self
    }

    pub fn with_delete<S, X>(mut self, setter: S) -> Self
    where
        S: IntoPyDeleterFunc<X>,
    {
        self.deleter = Some(setter.into_deleter());
        self
    }
}

#[pyimpl(with(SlotDescriptor))]
impl PyGetSet {
    // Descriptor methods

    #[pyslot]
    fn descr_set(
        zelf: PyObjectRef,
        obj: PyObjectRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let zelf = PyRef::<Self>::try_from_object(vm, zelf)?;
        match value {
            Some(value) => {
                if let Some(ref f) = zelf.setter {
                    f(vm, obj, value)
                } else {
                    Err(vm.new_attribute_error(format!(
                        "attribute '{}' of '{}' objects is not writable",
                        zelf.name,
                        obj.class().name
                    )))
                }
            }
            None => {
                if let Some(ref f) = zelf.deleter {
                    f(vm, obj)
                } else {
                    Err(vm.new_attribute_error(format!(
                        "attribute '{}' of '{}' objects is not writable",
                        zelf.name,
                        obj.class().name
                    )))
                }
            }
        }
    }
    #[pymethod]
    fn __set__(
        zelf: PyObjectRef,
        obj: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        Self::descr_set(zelf, obj, Some(value), vm)
    }
    #[pymethod]
    fn __delete__(zelf: PyObjectRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::descr_set(zelf, obj, None, vm)
    }

    #[pyproperty(magic)]
    fn name(&self) -> String {
        self.name.clone()
    }
}

pub(crate) fn init(context: &PyContext) {
    PyGetSet::extend_class(context, &context.types.getset_type);
}
