/*! Python `property` descriptor class.

*/
use super::PyType;
use crate::common::lock::PyRwLock;
use crate::object::PyAtomicRef;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::{ArgumentError, FromArgs, FuncArgs, Param, PySetterValue},
    types::{Constructor, GetDescriptor, Initializer},
};
use core::sync::atomic::{AtomicBool, Ordering};

struct SetNameArgs {
    _owner: PyObjectRef,
    name: PyObjectRef,
}

impl FromArgs for SetNameArgs {
    const PARAMS: Option<&'static [Param]> = Some(&[
        Param::positional_only("owner"),
        Param::positional_only("name"),
    ]);

    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let [owner, name]: [PyObjectRef; 2] =
            core::mem::take(&mut args.args)
                .try_into()
                .map_err(|args: Vec<PyObjectRef>| {
                    ArgumentError::Exception(vm.new_type_error(format!(
                        "__set_name__() takes 2 positional arguments but {} were given",
                        args.len()
                    )))
                })?;
        Ok(Self {
            _owner: owner,
            name,
        })
    }
}

#[pyclass(module = false, name = "property", traverse)]
#[derive(Debug)]
pub struct PyProperty {
    #[pymember(name = "fget")]
    getter: PyAtomicRef<Option<PyObject>>,
    #[pymember(name = "fset")]
    setter: PyAtomicRef<Option<PyObject>>,
    #[pymember(name = "fdel")]
    deleter: PyAtomicRef<Option<PyObject>>,
    #[pymember(name = "__doc__", writable)]
    doc: PyAtomicRef<Option<PyObject>>,
    name: PyRwLock<Option<PyObjectRef>>,
    #[pytraverse(skip)]
    getter_doc: core::sync::atomic::AtomicBool,
}

impl PyPayload for PyProperty {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.property_type
    }
}

#[derive(FromArgs)]
pub struct PropertyArgs {
    #[pyarg(any, optional)]
    fget: Option<PyObjectRef>,
    #[pyarg(any, optional)]
    fset: Option<PyObjectRef>,
    #[pyarg(any, optional)]
    fdel: Option<PyObjectRef>,
    #[pyarg(any, optional)]
    doc: Option<PyObjectRef>,
}

impl GetDescriptor for PyProperty {
    fn descr_get(
        zelf_obj: &PyObject,
        obj: Option<&PyObject>,
        _cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(zelf_obj, obj, vm)?;
        if vm.is_none(obj) {
            return Ok(zelf_obj.to_owned());
        }

        // Clone and release lock before calling Python code to prevent deadlock
        let value = zelf.getter.load_owned();
        if let Some(getter) = value {
            getter.call((obj.to_owned(),), vm)
        } else {
            let error_msg = zelf.format_property_error(obj, "getter", vm)?;
            Err(vm.new_attribute_error(error_msg))
        }
    }
}

impl PyProperty {
    fn set_property_name(&self, name: Option<PyObjectRef>) {
        // The replaced name can run a finalizer that accesses this property.
        let _old = core::mem::replace(&mut *self.name.write(), name);
    }

    // Helper method to get property name
    // Returns the name if available, None if not found, or propagates errors
    fn get_property_name(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
        // First check if name was set via __set_name__
        let value = self.name.read().clone();
        if let Some(name) = value {
            return Ok(Some(name));
        }

        // Clone and release lock before calling Python code to prevent deadlock
        let Some(getter) = self.getter.load_owned() else {
            return Ok(None);
        };

        vm.get_attribute_opt(&getter, "__name__")
    }

    pub(crate) fn get_fget(&self) -> Option<PyObjectRef> {
        self.getter.load_owned()
    }

    fn get_fset(&self) -> Option<PyObjectRef> {
        self.setter.load_owned()
    }

    fn get_fdel(&self) -> Option<PyObjectRef> {
        self.deleter.load_owned()
    }

    fn doc_getter(&self) -> Option<PyObjectRef> {
        self.doc.load_owned()
    }

    // Python builder functions

    // Helper method to create a new property with updated attributes
    fn clone_property_with(
        zelf: &Py<Self>,
        new_getter: Option<PyObjectRef>,
        new_setter: Option<PyObjectRef>,
        new_deleter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let getter = new_getter.or_else(|| zelf.get_fget());
        let setter = new_setter.or_else(|| zelf.get_fset());
        let deleter = new_deleter.or_else(|| zelf.get_fdel());
        let doc = if zelf.getter_doc.load(Ordering::Relaxed) && getter.is_some() {
            None
        } else {
            zelf.doc_getter()
        };
        let new_prop = zelf.class().as_object().call(
            (
                getter.unwrap_or_else(|| vm.ctx.none()),
                setter.unwrap_or_else(|| vm.ctx.none()),
                deleter.unwrap_or_else(|| vm.ctx.none()),
                doc.unwrap_or_else(|| vm.ctx.none()),
            ),
            vm,
        )?;

        // A subclass constructor may return an unrelated object. Read the old
        // name after that call, since the constructor can change it.
        if let Some(property) = new_prop.downcast_ref::<Self>() {
            let name = zelf.name.read().clone();
            property.set_property_name(name);
        }
        Ok(new_prop)
    }

    // Helper method to format property error messages
    #[cold]
    fn format_property_error(
        &self,
        obj: &PyObject,
        error_type: &str,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let prop_name = self.get_property_name(vm)?;
        let obj_type = obj.class();
        let qualname = obj_type.__qualname__(vm);

        match prop_name {
            Some(name) => Ok(format!(
                "property {} of {} object has no {}",
                name.repr(vm)?,
                qualname.repr(vm)?,
                error_type
            )),
            None => Ok(format!(
                "property of {} object has no {}",
                qualname.repr(vm)?,
                error_type
            )),
        }
    }
}

#[pyclass(
    with(Constructor, Initializer, GetDescriptor),
    flags(BASETYPE, HAS_WEAKREF)
)]
impl Py<PyProperty> {
    // Descriptor methods

    #[pyslot]
    fn descr_set(
        zelf: &PyObject,
        obj: PyObjectRef,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let zelf = zelf.try_to_ref::<PyProperty>(vm)?;
        match value {
            PySetterValue::Assign(value) => {
                // Clone and release lock before calling Python code to prevent deadlock
                let set = zelf.setter.load_owned();
                if let Some(setter) = set {
                    setter.call((obj, value), vm).map(drop)
                } else {
                    let error_msg = zelf.format_property_error(&obj, "setter", vm)?;
                    Err(vm.new_attribute_error(error_msg))
                }
            }
            PySetterValue::Delete => {
                // Clone and release lock before calling Python code to prevent deadlock
                let del = zelf.deleter.load_owned();
                if let Some(deleter) = del {
                    deleter.call((obj,), vm).map(drop)
                } else {
                    let error_msg = zelf.format_property_error(&obj, "deleter", vm)?;
                    Err(vm.new_attribute_error(error_msg))
                }
            }
        }
    }

    #[pygetset(name = "__name__")]
    fn name_getter(&self, vm: &VirtualMachine) -> PyResult {
        match self.get_property_name(vm)? {
            Some(name) => Ok(name),
            None => Err(vm.new_attribute_error("'property' object has no attribute '__name__'")),
        }
    }

    #[pygetset(name = "__name__", setter)]
    fn name_setter(&self, value: PySetterValue) {
        self.set_property_name(match value {
            PySetterValue::Assign(value) => Some(value),
            PySetterValue::Delete => None,
        });
    }

    #[pymethod]
    fn __set_name__(&self, SetNameArgs { name, .. }: SetNameArgs) {
        self.set_property_name(Some(name));
    }

    #[pymethod]
    fn getter(
        zelf: PyRef<PyProperty>,
        object: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        PyProperty::clone_property_with(&zelf, object, None, None, vm)
    }

    #[pymethod]
    fn setter(
        zelf: PyRef<PyProperty>,
        object: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        PyProperty::clone_property_with(&zelf, None, object, None, vm)
    }

    #[pymethod]
    fn deleter(
        zelf: PyRef<PyProperty>,
        object: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        PyProperty::clone_property_with(&zelf, None, None, object, vm)
    }

    #[pygetset]
    fn __isabstractmethod__(&self, vm: &VirtualMachine) -> PyResult {
        // Clone and release lock before calling Python code to prevent deadlock
        // Check getter
        if let Some(getter) = self.getter.load_owned()
            && getter.is_abstract(vm)?
        {
            return Ok(vm.ctx.new_bool(true).into());
        }

        // Check setter
        if let Some(setter) = self.setter.load_owned()
            && setter.is_abstract(vm)?
        {
            return Ok(vm.ctx.new_bool(true).into());
        }

        // Check deleter
        if let Some(deleter) = self.deleter.load_owned()
            && deleter.is_abstract(vm)?
        {
            return Ok(vm.ctx.new_bool(true).into());
        }

        Ok(vm.ctx.new_bool(false).into())
    }
}

impl Constructor for PyProperty {
    type Args = FuncArgs;

    fn py_new(_cls: &Py<PyType>, _args: FuncArgs, _vm: &VirtualMachine) -> PyResult<Self> {
        Ok(Self {
            getter: PyAtomicRef::new_empty(),
            setter: PyAtomicRef::new_empty(),
            deleter: PyAtomicRef::new_empty(),
            doc: PyAtomicRef::new_empty(),
            name: PyRwLock::new(None),
            getter_doc: AtomicBool::new(false),
        })
    }
}

impl Initializer for PyProperty {
    type Args = PropertyArgs;

    fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        let getter = args.fget.as_ref().filter(|value| !vm.is_none(value));
        let setter = args.fset.as_ref().filter(|value| !vm.is_none(value));
        let deleter = args.fdel.as_ref().filter(|value| !vm.is_none(value));

        // Publish each field before releasing its old owner. Finalizers can
        // reenter and observe the same partially initialized state as CPython.
        drop(zelf.getter.store(getter.cloned()));
        drop(zelf.setter.store(setter.cloned()));
        drop(zelf.deleter.store(deleter.cloned()));
        drop(zelf.doc.store(None));
        zelf.set_property_name(None);
        zelf.getter_doc.store(false, Ordering::Relaxed);

        let doc = match args
            .doc
            .as_ref()
            .filter(|value| !vm.is_none(value))
            .cloned()
        {
            Some(doc) => Some(doc),
            None => match getter {
                Some(getter) => {
                    let doc = vm
                        .get_attribute_opt(getter, "__doc__")?
                        .filter(|value| !vm.is_none(value));
                    if doc.is_some() {
                        zelf.getter_doc.store(true, Ordering::Relaxed);
                    }
                    doc
                }
                None => None,
            },
        };

        if zelf.class().is(vm.ctx.types.property_type) {
            drop(zelf.doc.store(doc));
        } else {
            // Subclasses may route this through an arbitrary descriptor.
            let doc = doc.unwrap_or_else(|| vm.ctx.none());
            match zelf.as_object().set_attr("__doc__", doc, vm) {
                Ok(()) => {}
                Err(e)
                    if !zelf.getter_doc.load(Ordering::Relaxed)
                        && e.fast_isinstance(vm.ctx.exceptions.attribute_error) => {}
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }
}

pub(crate) fn init(context: &'static Context) {
    PyProperty::extend_class(context, context.types.property_type);
}
