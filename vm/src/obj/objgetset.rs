/*! Python `attribute` descriptor class. (PyGetSet)

*/
use super::objtype::PyClassRef;
use crate::function::{OptionalArg, OwnedParam, RefParam};
use crate::pyobject::{
    IntoPyResult, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    TypeProtocol,
};
use crate::slots::SlotDescriptor;
use crate::vm::VirtualMachine;

pub type PyGetterFunc = Box<py_dyn_fn!(dyn Fn(&VirtualMachine, PyObjectRef) -> PyResult)>;
pub type PySetterFunc =
    Box<py_dyn_fn!(dyn Fn(&VirtualMachine, PyObjectRef, PyObjectRef) -> PyResult<()>)>;

pub trait IntoPyGetterFunc<T> {
    fn into_getter(self) -> PyGetterFunc;
}

impl<F, T, R> IntoPyGetterFunc<(OwnedParam<T>, R, VirtualMachine)> for F
where
    F: Fn(T, &VirtualMachine) -> R + 'static + Send + Sync,
    T: TryFromObject,
    R: IntoPyResult,
{
    fn into_getter(self) -> PyGetterFunc {
        Box::new(move |vm: &VirtualMachine, obj| {
            let obj = T::try_from_object(vm, obj)?;
            (self)(obj, vm).into_pyresult(vm)
        })
    }
}

impl<F, S, R> IntoPyGetterFunc<(RefParam<S>, R, VirtualMachine)> for F
where
    F: Fn(&S, &VirtualMachine) -> R + 'static + Send + Sync,
    S: PyValue,
    R: IntoPyResult,
{
    fn into_getter(self) -> PyGetterFunc {
        Box::new(move |vm: &VirtualMachine, obj| {
            let zelf = PyRef::<S>::try_from_object(vm, obj)?;
            (self)(&zelf, vm).into_pyresult(vm)
        })
    }
}

impl<F, T, R> IntoPyGetterFunc<(OwnedParam<T>, R)> for F
where
    F: Fn(T) -> R + 'static + Send + Sync,
    T: TryFromObject,
    R: IntoPyResult,
{
    fn into_getter(self) -> PyGetterFunc {
        IntoPyGetterFunc::into_getter(move |obj, _vm: &VirtualMachine| (self)(obj))
    }
}

impl<F, S, R> IntoPyGetterFunc<(RefParam<S>, R)> for F
where
    F: Fn(&S) -> R + 'static + Send + Sync,
    S: PyValue,
    R: IntoPyResult,
{
    fn into_getter(self) -> PyGetterFunc {
        IntoPyGetterFunc::into_getter(move |zelf: &S, _vm: &VirtualMachine| (self)(zelf))
    }
}

pub trait IntoPyNoResult {
    fn into_noresult(self) -> PyResult<()>;
}

impl IntoPyNoResult for () {
    fn into_noresult(self) -> PyResult<()> {
        Ok(())
    }
}

impl IntoPyNoResult for PyResult<()> {
    fn into_noresult(self) -> PyResult<()> {
        self
    }
}

pub trait IntoPySetterFunc<T> {
    fn into_setter(self) -> PySetterFunc;
}

impl<F, T, V, R> IntoPySetterFunc<(OwnedParam<T>, V, R, VirtualMachine)> for F
where
    F: Fn(T, V, &VirtualMachine) -> R + 'static + Send + Sync,
    T: TryFromObject,
    V: TryFromObject,
    R: IntoPyNoResult,
{
    fn into_setter(self) -> PySetterFunc {
        Box::new(move |vm: &VirtualMachine, obj, value| {
            let obj = T::try_from_object(vm, obj)?;
            let value = V::try_from_object(vm, value)?;
            (self)(obj, value, vm).into_noresult()
        })
    }
}

impl<F, S, V, R> IntoPySetterFunc<(RefParam<S>, V, R, VirtualMachine)> for F
where
    F: Fn(&S, V, &VirtualMachine) -> R + 'static + Send + Sync,
    S: PyValue,
    V: TryFromObject,
    R: IntoPyNoResult,
{
    fn into_setter(self) -> PySetterFunc {
        Box::new(move |vm: &VirtualMachine, obj, value| {
            let zelf = PyRef::<S>::try_from_object(vm, obj)?;
            let value = V::try_from_object(vm, value)?;
            (self)(&zelf, value, vm).into_noresult()
        })
    }
}

impl<F, T, V, R> IntoPySetterFunc<(OwnedParam<T>, V, R)> for F
where
    F: Fn(T, V) -> R + 'static + Send + Sync,
    T: TryFromObject,
    V: TryFromObject,
    R: IntoPyNoResult,
{
    fn into_setter(self) -> PySetterFunc {
        IntoPySetterFunc::into_setter(move |obj, v, _vm: &VirtualMachine| (self)(obj, v))
    }
}

impl<F, S, V, R> IntoPySetterFunc<(RefParam<S>, V, R)> for F
where
    F: Fn(&S, V) -> R + 'static + Send + Sync,
    S: PyValue,
    V: TryFromObject,
    R: IntoPyNoResult,
{
    fn into_setter(self) -> PySetterFunc {
        IntoPySetterFunc::into_setter(move |zelf: &S, v, _vm: &VirtualMachine| (self)(zelf, v))
    }
}

#[pyclass(module = false, name = "getset_descriptor")]
pub struct PyGetSet {
    name: String,
    getter: Option<PyGetterFunc>,
    setter: Option<PySetterFunc>,
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
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.getset_type.clone()
    }
}

pub type PyGetSetRef = PyRef<PyGetSet>;

impl SlotDescriptor for PyGetSet {
    fn descr_get(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: OptionalArg<PyObjectRef>,
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
    pub fn with_get<G, X>(name: String, getter: G) -> Self
    where
        G: IntoPyGetterFunc<X>,
    {
        Self {
            name,
            getter: Some(getter.into_getter()),
            setter: None,
        }
    }

    pub fn with_get_set<G, S, X, Y>(name: String, getter: G, setter: S) -> Self
    where
        G: IntoPyGetterFunc<X>,
        S: IntoPySetterFunc<Y>,
    {
        Self {
            name,
            getter: Some(getter.into_getter()),
            setter: Some(setter.into_setter()),
        }
    }
}

#[pyimpl(with(SlotDescriptor))]
impl PyGetSet {
    // Descriptor methods

    #[pymethod(magic)]
    fn set(&self, obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(ref f) = self.setter {
            f(vm, obj, value)
        } else {
            Err(vm.new_attribute_error(format!(
                "attribute '{}' of '{}' objects is not writable",
                self.name,
                obj.lease_class().name
            )))
        }
    }

    // TODO: give getset_descriptors names
    #[pyproperty(magic)]
    fn name(&self) {}
}

pub(crate) fn init(context: &PyContext) {
    PyGetSet::extend_class(context, &context.types.getset_type);
}
