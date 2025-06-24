/*! Python `property` descriptor class.

*/
use super::{PyStrRef, PyType, PyTypeRef};
use crate::common::lock::PyRwLock;
use crate::function::{IntoFuncArgs, PosArgs};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    function::{FuncArgs, PySetterValue},
    types::{Constructor, GetDescriptor, Initializer},
};
use std::sync::atomic::{AtomicBool, Ordering};

#[pyclass(module = false, name = "property", traverse)]
#[derive(Debug)]
pub struct PyProperty {
    getter: PyRwLock<Option<PyObjectRef>>,
    setter: PyRwLock<Option<PyObjectRef>>,
    deleter: PyRwLock<Option<PyObjectRef>>,
    doc: PyRwLock<Option<PyObjectRef>>,
    name: PyRwLock<Option<PyObjectRef>>,
    #[pytraverse(skip)]
    getter_doc: std::sync::atomic::AtomicBool,
}

impl PyPayload for PyProperty {
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
        } else if let Some(getter) = zelf.getter.read().as_ref() {
            getter.call((obj,), vm)
        } else {
            Err(vm.new_attribute_error("property has no getter".to_string()))
        }
    }
}

#[pyclass(with(Constructor, Initializer, GetDescriptor), flags(BASETYPE))]
impl PyProperty {
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
                if let Some(setter) = zelf.setter.read().as_ref() {
                    setter.call((obj, value), vm).map(drop)
                } else {
                    Err(vm.new_attribute_error("property has no setter".to_owned()))
                }
            }
            PySetterValue::Delete => {
                if let Some(deleter) = zelf.deleter.read().as_ref() {
                    deleter.call((obj,), vm).map(drop)
                } else {
                    Err(vm.new_attribute_error("property has no deleter".to_owned()))
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
        Self::descr_set(&zelf, obj, PySetterValue::Assign(value), vm)
    }
    #[pymethod]
    fn __delete__(zelf: PyObjectRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::descr_set(&zelf, obj, PySetterValue::Delete, vm)
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

    fn doc_getter(&self) -> Option<PyObjectRef> {
        self.doc.read().clone()
    }
    fn doc_setter(&self, value: Option<PyObjectRef>) {
        *self.doc.write() = value;
    }

    #[pymethod(magic)]
    fn set_name(&self, args: PosArgs, vm: &VirtualMachine) -> PyResult<()> {
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

    #[pymethod]
    fn getter(
        zelf: PyRef<Self>,
        getter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let new_getter = getter.or_else(|| zelf.fget());

        // Determine doc based on getter_doc flag
        let doc = if zelf.getter_doc.load(Ordering::Relaxed) && new_getter.is_some() {
            // If the original property uses getter doc and we have a new getter,
            // pass Py_None to let __init__ get the doc from the new getter
            Some(vm.ctx.none())
        } else {
            // Otherwise use the existing doc
            zelf.doc_getter()
        };

        // Create property args
        let args = PropertyArgs {
            fget: new_getter,
            fset: zelf.fset(),
            fdel: zelf.fdel(),
            doc,
            name: None,
        };

        // Create new property using py_new and init
        let new_prop = PyProperty::py_new(zelf.class().to_owned(), FuncArgs::default(), vm)?;
        let new_prop_ref = new_prop.downcast::<PyProperty>().unwrap();
        PyProperty::init(new_prop_ref.clone(), args, vm)?;

        // Copy the name if it exists
        if let Some(name) = zelf.name.read().clone() {
            *new_prop_ref.name.write() = Some(name);
        }

        Ok(new_prop_ref)
    }

    #[pymethod]
    fn setter(
        zelf: PyRef<Self>,
        setter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        // For setter, we need to preserve doc handling from the original property
        let doc = if zelf.getter_doc.load(Ordering::Relaxed) {
            // If original used getter_doc, pass None to let init get doc from getter
            Some(vm.ctx.none())
        } else {
            zelf.doc_getter()
        };

        let args = PropertyArgs {
            fget: zelf.fget(),
            fset: setter.or_else(|| zelf.fset()),
            fdel: zelf.fdel(),
            doc,
            name: None,
        };

        let new_prop = PyProperty::py_new(zelf.class().to_owned(), FuncArgs::default(), vm)?;
        let new_prop_ref = new_prop.downcast::<PyProperty>().unwrap();
        PyProperty::init(new_prop_ref.clone(), args, vm)?;

        // Copy the name if it exists
        if let Some(name) = zelf.name.read().clone() {
            *new_prop_ref.name.write() = Some(name);
        }

        Ok(new_prop_ref)
    }

    #[pymethod]
    fn deleter(
        zelf: PyRef<Self>,
        deleter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        // For deleter, we need to preserve doc handling from the original property
        let doc = if zelf.getter_doc.load(Ordering::Relaxed) {
            // If original used getter_doc, pass None to let init get doc from getter
            Some(vm.ctx.none())
        } else {
            zelf.doc_getter()
        };

        let args = PropertyArgs {
            fget: zelf.fget(),
            fset: zelf.fset(),
            fdel: deleter.or_else(|| zelf.fdel()),
            doc,
            name: None,
        };

        let new_prop = PyProperty::py_new(zelf.class().to_owned(), FuncArgs::default(), vm)?;
        let new_prop_ref = new_prop.downcast::<PyProperty>().unwrap();
        PyProperty::init(new_prop_ref.clone(), args, vm)?;

        // Copy the name if it exists
        if let Some(name) = zelf.name.read().clone() {
            *new_prop_ref.name.write() = Some(name);
        }

        Ok(new_prop_ref)
    }

    #[pygetset(magic)]
    fn isabstractmethod(&self, vm: &VirtualMachine) -> PyResult {
        // Check getter first
        if let Some(getter) = self.getter.read().as_ref() {
            if let Ok(isabstract) = getter.get_attr("__isabstractmethod__", vm) {
                let is_true = isabstract.try_to_bool(vm)?;
                if is_true {
                    return Ok(vm.ctx.new_bool(true).into());
                }
            }
        }

        // Check setter
        if let Some(setter) = self.setter.read().as_ref() {
            if let Ok(isabstract) = setter.get_attr("__isabstractmethod__", vm) {
                let is_true = isabstract.try_to_bool(vm)?;
                if is_true {
                    return Ok(vm.ctx.new_bool(true).into());
                }
            }
        }

        // Check deleter
        if let Some(deleter) = self.deleter.read().as_ref() {
            if let Ok(isabstract) = deleter.get_attr("__isabstractmethod__", vm) {
                let is_true = isabstract.try_to_bool(vm)?;
                if is_true {
                    return Ok(vm.ctx.new_bool(true).into());
                }
            }
        }

        Ok(vm.ctx.new_bool(false).into())
    }

    #[pygetset(magic, setter)]
    fn set_isabstractmethod(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(getter) = self.getter.read().to_owned() {
            getter.set_attr("__isabstractmethod__", value, vm)?;
        }
        Ok(())
    }
}

impl Constructor for PyProperty {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyProperty {
            getter: PyRwLock::new(None),
            setter: PyRwLock::new(None),
            deleter: PyRwLock::new(None),
            doc: PyRwLock::new(None),
            name: PyRwLock::new(None),
            getter_doc: AtomicBool::new(false),
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }
}

impl Initializer for PyProperty {
    type Args = PropertyArgs;

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        *zelf.getter.write() = args.fget.clone();
        *zelf.setter.write() = args.fset;
        *zelf.deleter.write() = args.fdel;
        *zelf.name.write() = args.name.map(|a| a.as_object().to_owned());

        // Set doc and getter_doc flag
        let mut getter_doc = false;

        // Helper to get doc from getter
        let get_getter_doc = |fget: &PyObjectRef| -> Option<PyObjectRef> {
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

        zelf.getter_doc.store(getter_doc, Ordering::Relaxed);

        Ok(())
    }
}

pub(crate) fn init(context: &Context) {
    PyProperty::extend_class(context, context.types.property_type);

    // This is a bit unfortunate, but this instance attribute overlaps with the
    // class __doc__ string..
    extend_class!(context, context.types.property_type, {
        "__doc__" => context.new_getset(
            "__doc__",
            context.types.property_type,
            PyProperty::doc_getter,
            PyProperty::doc_setter,
        ),
    });
}
