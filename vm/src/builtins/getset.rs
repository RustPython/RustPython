/*! Python `attribute` descriptor class. (PyGetSet)

*/
use super::{PyType, PyTypeRef};
use crate::{
    class::PyClassImpl,
    function::{IntoPyGetterFunc, IntoPySetterFunc, PyGetterFunc, PySetterFunc, PySetterValue},
    types::{GetDescriptor, Unconstructible},
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
};

#[pyclass(module = false, name = "getset_descriptor")]
pub struct PyGetSet {
    name: String,
    class: &'static Py<PyType>,
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

impl PyPayload for PyGetSet {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.getset_type
    }
}

impl GetDescriptor for PyGetSet {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = match Self::_check(&zelf, obj, vm) {
            Some(obj) => obj,
            None => return Ok(zelf),
        };
        if let Some(ref f) = zelf.getter {
            f(vm, obj)
        } else {
            Err(vm.new_attribute_error(format!(
                "attribute '{}' of '{}' objects is not readable",
                zelf.name,
                Self::class(&vm.ctx).name()
            )))
        }
    }
}

impl PyGetSet {
    pub fn new(name: String, class: &'static Py<PyType>) -> Self {
        Self {
            name,
            class,
            getter: None,
            setter: None,
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
}

#[pyclass(with(GetDescriptor, Unconstructible))]
impl PyGetSet {
    // Descriptor methods

    #[pyslot]
    fn descr_set(
        zelf: &PyObject,
        obj: PyObjectRef,
        value: PySetterValue<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let zelf = zelf.try_to_ref::<Self>(vm)?;
        if let Some(ref f) = zelf.setter {
            f(vm, obj, value)
        } else {
            Err(vm.new_attribute_error(format!(
                "attribute '{}' of '{}' objects is not writable",
                zelf.name,
                obj.class().name()
            )))
        }
    }
    #[pymethod]
    fn __set__(
        zelf: PyObjectRef,
        obj: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        Self::descr_set(&zelf, obj, PySetterValue::Assign(value), vm)
    }
    #[pymethod]
    fn __delete__(zelf: PyObjectRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::descr_set(&zelf, obj, PySetterValue::Delete, vm)
    }

    #[pygetset(magic)]
    fn name(&self) -> String {
        self.name.clone()
    }

    #[pygetset(magic)]
    fn qualname(&self) -> String {
        format!("{}.{}", self.class.slot_name(), self.name.clone())
    }

    #[pygetset(magic)]
    fn objclass(&self) -> PyTypeRef {
        self.class.to_owned()
    }
}
impl Unconstructible for PyGetSet {}

pub(crate) fn init(context: &Context) {
    PyGetSet::extend_class(context, context.types.getset_type);
}
