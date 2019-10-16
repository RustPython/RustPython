/*! Python `property` descriptor class.

*/
use std::cell::RefCell;

use super::objtype::PyClassRef;
use crate::function::{IntoPyNativeFunc, OptionalArg};
use crate::pyobject::{
    IdProtocol, PyClassImpl, PyContext, PyObject, PyObjectRef, PyRef, PyResult, PyValue,
    TypeProtocol,
};
use crate::vm::VirtualMachine;

// Read-only property, doesn't have __set__ or __delete__
#[pyclass]
#[derive(Debug)]
pub struct PyReadOnlyProperty {
    getter: PyObjectRef,
}

impl PyValue for PyReadOnlyProperty {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.readonly_property_type()
    }
}

pub type PyReadOnlyPropertyRef = PyRef<PyReadOnlyProperty>;

#[pyimpl]
impl PyReadOnlyProperty {
    #[pymethod(name = "__get__")]
    fn get(zelf: PyRef<Self>, obj: PyObjectRef, cls: PyClassRef, vm: &VirtualMachine) -> PyResult {
        if vm.is_none(&obj) {
            if cls.is(&vm.ctx.types.type_type) {
                vm.invoke(&zelf.getter, cls.into_object())
            } else {
                Ok(zelf.into_object())
            }
        } else {
            vm.invoke(&zelf.getter, obj)
        }
    }
}

/// Property attribute.
///
///   fget
///     function to be used for getting an attribute value
///   fset
///     function to be used for setting an attribute value
///   fdel
///     function to be used for del'ing an attribute
///   doc
///     docstring
///
/// Typical use is to define a managed attribute x:
///
/// class C(object):
///     def getx(self): return self._x
///     def setx(self, value): self._x = value
///     def delx(self): del self._x
///     x = property(getx, setx, delx, "I'm the 'x' property.")
///
/// Decorators make defining new properties or modifying existing ones easy:
///
/// class C(object):
///     @property
///     def x(self):
///         "I am the 'x' property."
///         return self._x
///     @x.setter
///     def x(self, value):
///         self._x = value
///     @x.deleter
///     def x(self):
///         del self._x
#[pyclass]
#[derive(Debug)]
pub struct PyProperty {
    getter: Option<PyObjectRef>,
    setter: Option<PyObjectRef>,
    deleter: Option<PyObjectRef>,
    doc: RefCell<Option<PyObjectRef>>,
}

impl PyValue for PyProperty {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.property_type()
    }
}

pub type PyPropertyRef = PyRef<PyProperty>;

#[derive(FromArgs)]
struct PropertyArgs {
    #[pyarg(positional_or_keyword, default = "None")]
    fget: Option<PyObjectRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    fset: Option<PyObjectRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    fdel: Option<PyObjectRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    doc: Option<PyObjectRef>,
}

#[pyimpl]
impl PyProperty {
    #[pyslot(new)]
    fn tp_new(cls: PyClassRef, args: PropertyArgs, vm: &VirtualMachine) -> PyResult<PyPropertyRef> {
        PyProperty {
            getter: args.fget,
            setter: args.fset,
            deleter: args.fdel,
            doc: RefCell::new(args.doc),
        }
        .into_ref_with_type(vm, cls)
    }

    // Descriptor methods

    // specialised version that doesn't check for None
    pub(crate) fn instance_binding_get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(ref getter) = self.getter.as_ref() {
            vm.invoke(getter, obj)
        } else {
            Err(vm.new_attribute_error("unreadable attribute".to_string()))
        }
    }

    #[pymethod(name = "__get__")]
    fn get(
        zelf: PyRef<Self>,
        obj: PyObjectRef,
        _owner: OptionalArg<PyClassRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if let Some(getter) = zelf.getter.as_ref() {
            if obj.is(vm.ctx.none.as_object()) {
                Ok(zelf.into_object())
            } else {
                vm.invoke(&getter, obj)
            }
        } else {
            Err(vm.new_attribute_error("unreadable attribute".to_string()))
        }
    }

    #[pymethod(name = "__set__")]
    fn set(&self, obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(ref setter) = self.setter.as_ref() {
            vm.invoke(setter, vec![obj, value])
        } else {
            Err(vm.new_attribute_error("can't set attribute".to_string()))
        }
    }

    #[pymethod(name = "__delete__")]
    fn delete(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(ref deleter) = self.deleter.as_ref() {
            vm.invoke(deleter, obj)
        } else {
            Err(vm.new_attribute_error("can't delete attribute".to_string()))
        }
    }

    // Access functions

    #[pyproperty]
    fn fget(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.getter.clone()
    }

    #[pyproperty]
    fn fset(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.setter.clone()
    }

    #[pyproperty]
    fn fdel(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.deleter.clone()
    }

    fn doc_getter(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.doc.borrow().clone()
    }

    fn doc_setter(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.doc.replace(py_none_to_option(vm, &value));
        Ok(vm.get_none())
    }

    // Python builder functions

    #[pymethod]
    fn getter(
        zelf: PyRef<Self>,
        getter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyPropertyRef> {
        PyProperty {
            getter: getter.or_else(|| zelf.getter.clone()),
            setter: zelf.setter.clone(),
            deleter: zelf.deleter.clone(),
            doc: RefCell::new(None),
        }
        .into_ref_with_type(vm, TypeProtocol::class(&zelf))
    }

    #[pymethod]
    fn setter(
        zelf: PyRef<Self>,
        setter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyPropertyRef> {
        PyProperty {
            getter: zelf.getter.clone(),
            setter: setter.or_else(|| zelf.setter.clone()),
            deleter: zelf.deleter.clone(),
            doc: RefCell::new(None),
        }
        .into_ref_with_type(vm, TypeProtocol::class(&zelf))
    }

    #[pymethod]
    fn deleter(
        zelf: PyRef<Self>,
        deleter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyPropertyRef> {
        PyProperty {
            getter: zelf.getter.clone(),
            setter: zelf.setter.clone(),
            deleter: deleter.or_else(|| zelf.deleter.clone()),
            doc: RefCell::new(None),
        }
        .into_ref_with_type(vm, TypeProtocol::class(&zelf))
    }
}

/// Take a python object and turn it into an option object, where python None maps to rust None.
fn py_none_to_option(vm: &VirtualMachine, value: &PyObjectRef) -> Option<PyObjectRef> {
    if vm.ctx.none().is(value) {
        None
    } else {
        Some(value.clone())
    }
}

pub struct PropertyBuilder<'a> {
    ctx: &'a PyContext,
    getter: Option<PyObjectRef>,
    setter: Option<PyObjectRef>,
}

impl<'a> PropertyBuilder<'a> {
    pub fn new(ctx: &'a PyContext) -> Self {
        Self {
            ctx,
            getter: None,
            setter: None,
        }
    }

    pub fn add_getter<I, V, F: IntoPyNativeFunc<I, V>>(self, func: F) -> Self {
        let func = self.ctx.new_rustfunc(func);
        Self {
            ctx: self.ctx,
            getter: Some(func),
            setter: self.setter,
        }
    }

    pub fn add_setter<I, V, F: IntoPyNativeFunc<(I, V), PyResult>>(self, func: F) -> Self {
        let func = self.ctx.new_rustfunc(func);
        Self {
            ctx: self.ctx,
            getter: self.getter,
            setter: Some(func),
        }
    }

    pub fn create(self) -> PyObjectRef {
        if self.setter.is_some() {
            let payload = PyProperty {
                getter: self.getter.clone(),
                setter: self.setter.clone(),
                deleter: None,
                doc: RefCell::new(None),
            };

            PyObject::new(payload, self.ctx.property_type(), None)
        } else {
            let payload = PyReadOnlyProperty {
                getter: self.getter.expect(
                    "One of add_getter/add_setter must be called when constructing a property",
                ),
            };

            PyObject::new(payload, self.ctx.readonly_property_type(), None)
        }
    }
}

pub fn init(context: &PyContext) {
    PyReadOnlyProperty::extend_class(context, &context.types.readonly_property_type);
    PyProperty::extend_class(context, &context.types.property_type);

    // This is a bit unfortunate, but this instance attribute overlaps with the
    // class __doc__ string..
    extend_class!(context, &context.types.property_type, {
        "__doc__" =>
        PropertyBuilder::new(context)
            .add_getter(PyProperty::doc_getter)
            .add_setter(PyProperty::doc_setter)
            .create(),
    });
}
