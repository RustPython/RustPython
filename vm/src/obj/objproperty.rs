/*! Python `property` descriptor class.

*/

use crate::function::{IntoPyNativeFunc, OptionalArg, PyFuncArgs};
use crate::obj::objtype::PyClassRef;
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
    fn get(
        zelf: PyRef<Self>,
        obj: PyObjectRef,
        _owner: OptionalArg<PyClassRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if obj.is(vm.ctx.none.as_object()) {
            Ok(zelf.into_object())
        } else {
            vm.invoke(zelf.getter.clone(), obj)
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
    doc: Option<PyObjectRef>,
}

impl PyValue for PyProperty {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.property_type()
    }
}

pub type PyPropertyRef = PyRef<PyProperty>;

#[pyimpl]
impl PyProperty {
    #[pymethod(name = "__new__")]
    fn new_property(
        cls: PyClassRef,
        args: PyFuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyPropertyRef> {
        arg_check!(
            vm,
            args,
            required = [],
            optional = [(fget, None), (fset, None), (fdel, None), (doc, None)]
        );

        fn into_option(vm: &VirtualMachine, arg: Option<&PyObjectRef>) -> Option<PyObjectRef> {
            arg.and_then(|arg| {
                if vm.ctx.none().is(arg) {
                    None
                } else {
                    Some(arg.clone())
                }
            })
        }

        PyProperty {
            getter: into_option(vm, fget),
            setter: into_option(vm, fset),
            deleter: into_option(vm, fdel),
            doc: into_option(vm, doc),
        }
        .into_ref_with_type(vm, cls)
    }

    // Descriptor methods

    // specialised version that doesn't check for None
    pub(crate) fn instance_binding_get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(getter) = self.getter.as_ref() {
            vm.invoke(getter.clone(), obj)
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
                vm.invoke(getter.clone(), obj)
            }
        } else {
            Err(vm.new_attribute_error("unreadable attribute".to_string()))
        }
    }

    #[pymethod(name = "__set__")]
    fn set(&self, obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(setter) = self.setter.as_ref() {
            vm.invoke(setter.clone(), vec![obj, value])
        } else {
            Err(vm.new_attribute_error("can't set attribute".to_string()))
        }
    }

    #[pymethod(name = "__delete__")]
    fn delete(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(deleter) = self.deleter.as_ref() {
            vm.invoke(deleter.clone(), obj)
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
            doc: None,
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
            doc: None,
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
            doc: None,
        }
        .into_ref_with_type(vm, TypeProtocol::class(&zelf))
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
                doc: None,
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
    PyReadOnlyProperty::extend_class(context, &context.readonly_property_type);
    PyProperty::extend_class(context, &context.property_type);
}
