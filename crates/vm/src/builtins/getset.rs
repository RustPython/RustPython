/*! Python `attribute` descriptor class. (PyGetSet)

*/
use super::PyType;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::type_::PointerSlot,
    class::PyClassImpl,
    function::{IntoPyGetterFunc, IntoPySetterFunc, PyGetterFunc, PySetterFunc, PySetterValue},
    types::{GetDescriptor, Representable},
};

#[pyclass(module = false, name = "getset_descriptor")]
pub struct PyGetSet {
    name: String,
    class: PointerSlot<Py<PyType>>, // A class type freed before getset is non-sense.
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
    #[inline]
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
            class: PointerSlot::from(class),
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

#[pyclass(flags(DISALLOW_INSTANTIATION), with(GetDescriptor, Representable))]
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

    #[pygetset]
    fn __name__(&self) -> String {
        self.name.clone()
    }

    #[pygetset]
    fn __qualname__(&self) -> String {
        format!(
            "{}.{}",
            unsafe { self.class.borrow_static() }.slot_name(),
            self.name.clone()
        )
    }

    #[pymember]
    fn __objclass__(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf: &Py<Self> = zelf.try_to_value(vm)?;
        Ok(unsafe { zelf.class.borrow_static() }.to_owned().into())
    }
}

impl Representable for PyGetSet {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let class = unsafe { zelf.class.borrow_static() };
        // Special case for object type
        if std::ptr::eq(class, vm.ctx.types.object_type) {
            Ok(format!("<attribute '{}'>", zelf.name))
        } else {
            Ok(format!(
                "<attribute '{}' of '{}' objects>",
                zelf.name,
                class.name()
            ))
        }
    }
}

pub(crate) fn init(context: &Context) {
    PyGetSet::extend_class(context, context.types.getset_type);
}
