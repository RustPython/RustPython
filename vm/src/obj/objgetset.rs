/*! Python `attribute` descriptor class. (PyGetSet)

*/
use super::objtype::PyClassRef;
use crate::function::{OptionalArg, OwnedParam, RefParam};
use crate::pyobject::{
    IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
};
use crate::slots::PyBuiltinDescriptor;
use crate::vm::VirtualMachine;

pub type PyGetterFunc = Box<dyn Fn(&VirtualMachine, PyObjectRef) -> PyResult>;
pub type PySetterFunc = Box<dyn Fn(&VirtualMachine, PyObjectRef, PyObjectRef) -> PyResult<()>>;

pub trait IntoPyGetterFunc<T, R> {
    fn into_getter(self) -> PyGetterFunc;
}

impl<F, T, R> IntoPyGetterFunc<OwnedParam<T>, R> for F
where
    F: Fn(T, &VirtualMachine) -> R + 'static,
    T: TryFromObject,
    R: IntoPyObject,
{
    fn into_getter(self) -> PyGetterFunc {
        Box::new(move |vm, obj| {
            let obj = T::try_from_object(vm, obj)?;
            (self)(obj, vm).into_pyobject(vm)
        })
    }
}

impl<F, S, R> IntoPyGetterFunc<RefParam<S>, R> for F
where
    F: Fn(&S, &VirtualMachine) -> R + 'static,
    S: PyValue,
    R: IntoPyObject,
{
    fn into_getter(self) -> PyGetterFunc {
        Box::new(move |vm, obj| {
            let zelf = PyRef::<S>::try_from_object(vm, obj)?;
            (self)(&zelf, vm).into_pyobject(vm)
        })
    }
}

pub trait IntoPySetterFunc<T, V> {
    fn into_setter(self) -> PySetterFunc;
}

impl<F, T, V> IntoPySetterFunc<OwnedParam<T>, V> for F
where
    F: Fn(T, V, &VirtualMachine) -> PyResult<()> + 'static,
    T: TryFromObject,
    V: TryFromObject,
{
    fn into_setter(self) -> PySetterFunc {
        Box::new(move |vm, obj, value| {
            let obj = T::try_from_object(vm, obj)?;
            let value = V::try_from_object(vm, value)?;
            (self)(obj, value, vm)
        })
    }
}

impl<F, S, V> IntoPySetterFunc<RefParam<S>, V> for F
where
    F: Fn(&S, V, &VirtualMachine) -> PyResult<()> + 'static,
    S: PyValue,
    V: TryFromObject,
{
    fn into_setter(self) -> PySetterFunc {
        Box::new(move |vm, obj, value| {
            let zelf = PyRef::<S>::try_from_object(vm, obj)?;
            let value = V::try_from_object(vm, value)?;
            (self)(&zelf, value, vm)
        })
    }
}

#[pyclass]
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
        vm.ctx.getset_type()
    }
}

pub type PyGetSetRef = PyRef<PyGetSet>;

impl PyBuiltinDescriptor for PyGetSet {
    fn get(
        zelf: PyRef<Self>,
        obj: PyObjectRef,
        _cls: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
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
    pub fn with_get<G, T, R>(name: String, getter: G) -> Self
    where
        G: IntoPyGetterFunc<T, R>,
    {
        Self {
            name,
            getter: Some(getter.into_getter()),
            setter: None,
        }
    }

    pub fn with_get_set<G, S, GT, GR, ST, SV>(name: String, getter: G, setter: S) -> Self
    where
        G: IntoPyGetterFunc<GT, GR>,
        S: IntoPySetterFunc<ST, SV>,
    {
        Self {
            name,
            getter: Some(getter.into_getter()),
            setter: Some(setter.into_setter()),
        }
    }
}

#[pyimpl(with(PyBuiltinDescriptor))]
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
                Self::class(vm).name
            )))
        }
    }
}

pub(crate) fn init(context: &PyContext) {
    PyGetSet::extend_class(context, &context.types.getset_type);
}
