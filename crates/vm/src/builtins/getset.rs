//! Python `attribute` descriptor class. (PyGetSet)

use super::PyType;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::{IntoPyGetterFunc, IntoPySetterFunc, PyGetterFunc, PySetterFunc, PySetterValue},
    object::{Traverse, TraverseFn},
    types::{GetDescriptor, Representable},
};

#[pyclass(module = false, name = "getset_descriptor", traverse = "manual")]
pub struct PyGetSet {
    name: String,
    /// `d_type`. Owned: a type's namespace can outlive the type, and the
    /// descriptors it holds have to stay valid for as long as it does.
    class: PyRef<PyType>,
    getter: Option<PyGetterFunc>,
    setter: Option<PySetterFunc>,
    // doc: Option<String>,
}

impl core::fmt::Debug for PyGetSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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

// Only `class` is traced: the getter and setter closures are plain functions.
unsafe impl Traverse for PyGetSet {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.class.traverse(tracer_fn);
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
    #[must_use]
    pub fn new(name: &str, class: &Py<PyType>) -> Self {
        Self {
            name: name.into(),
            class: class.to_owned(),
            getter: None,
            setter: None,
        }
    }

    #[must_use]
    pub fn with_get<G, X>(mut self, getter: G) -> Self
    where
        G: IntoPyGetterFunc<X>,
    {
        self.getter = Some(getter.into_getter());
        self
    }

    #[must_use]
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

    #[pygetset]
    fn __name__(&self) -> String {
        self.name.clone()
    }

    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.class.slot_name(), self.name.clone())
    }

    #[pymember]
    fn __objclass__(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf: &Py<Self> = zelf.try_to_value(vm)?;
        Ok(zelf.class.clone().into())
    }
}

impl Representable for PyGetSet {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let class = &zelf.class;
        // Special case for object type
        if class.is(vm.ctx.types.object_type) {
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

pub(crate) fn init(context: &'static Context) {
    PyGetSet::extend_class(context, context.types.getset_type);
}
