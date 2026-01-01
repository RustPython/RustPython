/*! Python `property` descriptor class.

*/
use super::{PyStrRef, PyType};
use crate::common::lock::PyRwLock;
use crate::function::{IntoFuncArgs, PosArgs};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::{FuncArgs, PySetterValue},
    types::{Constructor, GetDescriptor, Initializer},
};
use core::sync::atomic::{AtomicBool, Ordering};

#[pyclass(module = false, name = "property", traverse)]
#[derive(Debug)]
pub struct PyProperty {
    getter: PyRwLock<Option<PyObjectRef>>,
    setter: PyRwLock<Option<PyObjectRef>>,
    deleter: PyRwLock<Option<PyObjectRef>>,
    doc: PyRwLock<Option<PyObjectRef>>,
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
    #[pyarg(any, default)]
    fget: Option<PyObjectRef>,
    #[pyarg(any, default)]
    fset: Option<PyObjectRef>,
    #[pyarg(any, default)]
    fdel: Option<PyObjectRef>,
    #[pyarg(any, default)]
    doc: Option<PyObjectRef>,
    #[pyarg(any, default)]
    name: Option<PyStrRef>,
}

impl GetDescriptor for PyProperty {
    fn descr_get(
        zelf_obj: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(&zelf_obj, obj, vm)?;
        if vm.is_none(&obj) {
            Ok(zelf_obj)
        } else if let Some(getter) = zelf.getter.read().clone() {
            // Clone and release lock before calling Python code to prevent deadlock
            getter.call((obj,), vm)
        } else {
            let error_msg = zelf.format_property_error(&obj, "getter", vm)?;
            Err(vm.new_attribute_error(error_msg))
        }
    }
}

#[pyclass(with(Constructor, Initializer, GetDescriptor), flags(BASETYPE))]
impl PyProperty {
    // Helper method to get property name
    // Returns the name if available, None if not found, or propagates errors
    fn get_property_name(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
        // First check if name was set via __set_name__
        if let Some(name) = self.name.read().clone() {
            return Ok(Some(name));
        }

        // Clone and release lock before calling Python code to prevent deadlock
        let Some(getter) = self.getter.read().clone() else {
            return Ok(None);
        };

        match getter.get_attr("__name__", vm) {
            Ok(name) => Ok(Some(name)),
            Err(e) => {
                // If it's an AttributeError from the getter, return None
                // Otherwise, propagate the original exception (e.g., RuntimeError)
                if e.class().is(vm.ctx.exceptions.attribute_error) {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }

    // Descriptor methods

    #[pyslot]
    fn descr_set(
        zelf: &PyObject,
        obj: PyObjectRef,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let zelf = zelf.try_to_ref::<Self>(vm)?;
        match value {
            PySetterValue::Assign(value) => {
                // Clone and release lock before calling Python code to prevent deadlock
                if let Some(setter) = zelf.setter.read().clone() {
                    setter.call((obj, value), vm).map(drop)
                } else {
                    let error_msg = zelf.format_property_error(&obj, "setter", vm)?;
                    Err(vm.new_attribute_error(error_msg))
                }
            }
            PySetterValue::Delete => {
                // Clone and release lock before calling Python code to prevent deadlock
                if let Some(deleter) = zelf.deleter.read().clone() {
                    deleter.call((obj,), vm).map(drop)
                } else {
                    let error_msg = zelf.format_property_error(&obj, "deleter", vm)?;
                    Err(vm.new_attribute_error(error_msg))
                }
            }
        }
    }

    // Access functions

    #[pygetset]
    fn fget(&self) -> Option<PyObjectRef> {
        self.getter.read().clone()
    }

    #[pygetset]
    fn fset(&self) -> Option<PyObjectRef> {
        self.setter.read().clone()
    }

    #[pygetset]
    fn fdel(&self) -> Option<PyObjectRef> {
        self.deleter.read().clone()
    }

    #[pygetset(name = "__name__")]
    fn name_getter(&self, vm: &VirtualMachine) -> PyResult {
        match self.get_property_name(vm)? {
            Some(name) => Ok(name),
            None => Err(
                vm.new_attribute_error("'property' object has no attribute '__name__'".to_owned())
            ),
        }
    }

    #[pygetset(name = "__name__", setter)]
    fn name_setter(&self, value: PyObjectRef) {
        *self.name.write() = Some(value);
    }

    fn doc_getter(&self) -> Option<PyObjectRef> {
        self.doc.read().clone()
    }
    fn doc_setter(&self, value: Option<PyObjectRef>) {
        *self.doc.write() = value;
    }

    #[pymethod]
    fn __set_name__(&self, args: PosArgs, vm: &VirtualMachine) -> PyResult<()> {
        let func_args = args.into_args(vm);
        let func_args_len = func_args.args.len();
        let (_owner, name): (PyObjectRef, PyObjectRef) = func_args.bind(vm).map_err(|_e| {
            vm.new_type_error(format!(
                "__set_name__() takes 2 positional arguments but {func_args_len} were given"
            ))
        })?;

        *self.name.write() = Some(name);

        Ok(())
    }

    // Python builder functions

    // Helper method to create a new property with updated attributes
    fn clone_property_with(
        zelf: PyRef<Self>,
        new_getter: Option<PyObjectRef>,
        new_setter: Option<PyObjectRef>,
        new_deleter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        // Determine doc based on getter_doc flag and whether we're updating the getter
        let doc = if zelf.getter_doc.load(Ordering::Relaxed) && new_getter.is_some() {
            // If the original property uses getter doc and we have a new getter,
            // pass Py_None to let __init__ get the doc from the new getter
            Some(vm.ctx.none())
        } else if zelf.getter_doc.load(Ordering::Relaxed) {
            // If original used getter_doc but we're not changing the getter,
            // pass None to let init get doc from existing getter
            Some(vm.ctx.none())
        } else {
            // Otherwise use the existing doc
            zelf.doc_getter()
        };

        // Create property args with updated values
        let args = PropertyArgs {
            fget: new_getter.or_else(|| zelf.fget()),
            fset: new_setter.or_else(|| zelf.fset()),
            fdel: new_deleter.or_else(|| zelf.fdel()),
            doc,
            name: None,
        };

        // Create new property using py_new and init
        let new_prop = Self::slot_new(zelf.class().to_owned(), FuncArgs::default(), vm)?;
        let new_prop_ref = new_prop.downcast::<Self>().unwrap();
        Self::init(new_prop_ref.clone(), args, vm)?;

        // Copy the name if it exists
        if let Some(name) = zelf.name.read().clone() {
            *new_prop_ref.name.write() = Some(name);
        }

        Ok(new_prop_ref)
    }

    #[pymethod]
    fn getter(
        zelf: PyRef<Self>,
        getter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        Self::clone_property_with(zelf, getter, None, None, vm)
    }

    #[pymethod]
    fn setter(
        zelf: PyRef<Self>,
        setter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        Self::clone_property_with(zelf, None, setter, None, vm)
    }

    #[pymethod]
    fn deleter(
        zelf: PyRef<Self>,
        deleter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        Self::clone_property_with(zelf, None, None, deleter, vm)
    }

    #[pygetset]
    fn __isabstractmethod__(&self, vm: &VirtualMachine) -> PyResult {
        // Helper to check if a method is abstract
        let is_abstract = |method: &PyObject| -> PyResult<bool> {
            match method.get_attr("__isabstractmethod__", vm) {
                Ok(isabstract) => isabstract.try_to_bool(vm),
                Err(_) => Ok(false),
            }
        };

        // Clone and release lock before calling Python code to prevent deadlock
        // Check getter
        if let Some(getter) = self.getter.read().clone()
            && is_abstract(&getter)?
        {
            return Ok(vm.ctx.new_bool(true).into());
        }

        // Check setter
        if let Some(setter) = self.setter.read().clone()
            && is_abstract(&setter)?
        {
            return Ok(vm.ctx.new_bool(true).into());
        }

        // Check deleter
        if let Some(deleter) = self.deleter.read().clone()
            && is_abstract(&deleter)?
        {
            return Ok(vm.ctx.new_bool(true).into());
        }

        Ok(vm.ctx.new_bool(false).into())
    }

    #[pygetset(setter)]
    fn set___isabstractmethod__(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Clone and release lock before calling Python code to prevent deadlock
        if let Some(getter) = self.getter.read().clone() {
            getter.set_attr("__isabstractmethod__", value, vm)?;
        }
        Ok(())
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

impl Constructor for PyProperty {
    type Args = FuncArgs;

    fn py_new(_cls: &Py<PyType>, _args: FuncArgs, _vm: &VirtualMachine) -> PyResult<Self> {
        Ok(Self {
            getter: PyRwLock::new(None),
            setter: PyRwLock::new(None),
            deleter: PyRwLock::new(None),
            doc: PyRwLock::new(None),
            name: PyRwLock::new(None),
            getter_doc: AtomicBool::new(false),
        })
    }
}

impl Initializer for PyProperty {
    type Args = PropertyArgs;

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // Set doc and getter_doc flag
        let mut getter_doc = false;

        // Helper to get doc from getter
        let get_getter_doc = |fget: &PyObject| -> Option<PyObjectRef> {
            fget.get_attr("__doc__", vm)
                .ok()
                .filter(|doc| !vm.is_none(doc))
        };

        let doc = match args.doc {
            Some(doc) if !vm.is_none(&doc) => Some(doc),
            _ => {
                // No explicit doc or doc is None, try to get from getter
                args.fget.as_ref().and_then(|fget| {
                    get_getter_doc(fget).inspect(|_| {
                        getter_doc = true;
                    })
                })
            }
        };

        // Check if this is a property subclass
        let is_exact_property = zelf.class().is(vm.ctx.types.property_type);

        if is_exact_property {
            // For exact property type, store doc in the field
            *zelf.doc.write() = doc;
        } else {
            // For property subclass, set __doc__ as an attribute
            let doc_to_set = doc.unwrap_or_else(|| vm.ctx.none());
            match zelf.as_object().set_attr("__doc__", doc_to_set, vm) {
                Ok(()) => {}
                Err(e) if !getter_doc && e.class().is(vm.ctx.exceptions.attribute_error) => {
                    // Silently ignore AttributeError for backwards compatibility
                    // (only when not using getter_doc)
                }
                Err(e) => return Err(e),
            }
        }

        *zelf.getter.write() = args.fget;
        *zelf.setter.write() = args.fset;
        *zelf.deleter.write() = args.fdel;
        *zelf.name.write() = args.name.map(|a| a.as_object().to_owned());
        zelf.getter_doc.store(getter_doc, Ordering::Relaxed);

        Ok(())
    }
}

pub(crate) fn init(context: &Context) {
    PyProperty::extend_class(context, context.types.property_type);

    // This is a bit unfortunate, but this instance attribute overlaps with the
    // class __doc__ string..
    extend_class!(context, context.types.property_type, {
        "__doc__" => context.new_static_getset(
            "__doc__",
            context.types.property_type,
            PyProperty::doc_getter,
            PyProperty::doc_setter,
        ),
    });
}
