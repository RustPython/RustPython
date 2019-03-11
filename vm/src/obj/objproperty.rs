/*! Python `property` descriptor class.

*/

use std::marker::PhantomData;

use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    IntoPyNativeFunc, OptionalArg, PyContext, PyObject, PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::VirtualMachine;

/// Read-only property, doesn't have __set__ or __delete__
#[derive(Debug)]
pub struct PyReadOnlyProperty {
    getter: PyObjectRef,
}

impl PyValue for PyReadOnlyProperty {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.readonly_property_type()
    }
}

pub type PyReadOnlyPropertyRef = PyRef<PyReadOnlyProperty>;

impl PyReadOnlyPropertyRef {
    fn get(self, obj: PyObjectRef, _owner: PyClassRef, vm: &mut VirtualMachine) -> PyResult {
        vm.invoke(self.getter.clone(), obj)
    }
}

/// Fully fledged property
#[derive(Debug)]
pub struct PyProperty {
    getter: Option<PyObjectRef>,
    setter: Option<PyObjectRef>,
    deleter: Option<PyObjectRef>,
}

impl PyValue for PyProperty {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.property_type()
    }
}

pub type PyPropertyRef = PyRef<PyProperty>;

impl PyPropertyRef {
    fn new_property(
        cls: PyClassRef,
        fget: OptionalArg<PyObjectRef>,
        fset: OptionalArg<PyObjectRef>,
        fdel: OptionalArg<PyObjectRef>,
        _doc: OptionalArg<PyStringRef>,
        vm: &mut VirtualMachine,
    ) -> PyResult<PyPropertyRef> {
        Self::new_with_type(
            vm,
            PyProperty {
                getter: fget.into_option(),
                setter: fset.into_option(),
                deleter: fdel.into_option(),
            },
            cls,
        )
    }

    // Descriptor methods

    fn get(self, obj: PyObjectRef, _owner: PyClassRef, vm: &mut VirtualMachine) -> PyResult {
        if let Some(getter) = self.getter.as_ref() {
            vm.invoke(getter.clone(), obj)
        } else {
            Err(vm.new_attribute_error("unreadable attribute".to_string()))
        }
    }

    fn set(self, obj: PyObjectRef, value: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if let Some(setter) = self.setter.as_ref() {
            vm.invoke(setter.clone(), vec![obj, value])
        } else {
            Err(vm.new_attribute_error("can't set attribute".to_string()))
        }
    }

    fn delete(self, obj: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        if let Some(deleter) = self.deleter.as_ref() {
            vm.invoke(deleter.clone(), obj)
        } else {
            Err(vm.new_attribute_error("can't delete attribute".to_string()))
        }
    }

    // Access functions

    fn fget(self, _vm: &mut VirtualMachine) -> Option<PyObjectRef> {
        self.getter.clone()
    }

    fn fset(self, _vm: &mut VirtualMachine) -> Option<PyObjectRef> {
        self.setter.clone()
    }

    fn fdel(self, _vm: &mut VirtualMachine) -> Option<PyObjectRef> {
        self.deleter.clone()
    }

    // Python builder functions

    fn getter(self, getter: Option<PyObjectRef>, vm: &mut VirtualMachine) -> PyResult<Self> {
        Self::new_with_type(
            vm,
            PyProperty {
                getter: getter.or_else(|| self.getter.clone()),
                setter: self.setter.clone(),
                deleter: self.deleter.clone(),
            },
            self.typ(),
        )
    }

    fn setter(self, setter: Option<PyObjectRef>, vm: &mut VirtualMachine) -> PyResult<Self> {
        Self::new_with_type(
            vm,
            PyProperty {
                getter: self.getter.clone(),
                setter: setter.or_else(|| self.setter.clone()),
                deleter: self.deleter.clone(),
            },
            self.typ(),
        )
    }

    fn deleter(self, deleter: Option<PyObjectRef>, vm: &mut VirtualMachine) -> PyResult<Self> {
        Self::new_with_type(
            vm,
            PyProperty {
                getter: self.getter.clone(),
                setter: self.setter.clone(),
                deleter: deleter.or_else(|| self.deleter.clone()),
            },
            self.typ(),
        )
    }
}

pub struct PropertyBuilder<'a, T> {
    ctx: &'a PyContext,
    getter: Option<PyObjectRef>,
    setter: Option<PyObjectRef>,
    _return: PhantomData<T>,
}

impl<'a, T> PropertyBuilder<'a, T> {
    pub fn new(ctx: &'a PyContext) -> Self {
        Self {
            ctx,
            getter: None,
            setter: None,
            _return: PhantomData,
        }
    }

    pub fn add_getter<I, F: IntoPyNativeFunc<I, T>>(self, func: F) -> Self {
        let func = self.ctx.new_rustfunc(func);
        Self {
            ctx: self.ctx,
            getter: Some(func),
            setter: self.setter,
            _return: PhantomData,
        }
    }

    pub fn add_setter<I, V, F: IntoPyNativeFunc<(I, V), PyResult>>(self, func: F) -> Self {
        let func = self.ctx.new_rustfunc(func);
        Self {
            ctx: self.ctx,
            getter: self.getter,
            setter: Some(func),
            _return: PhantomData,
        }
    }

    pub fn create(self) -> PyObjectRef {
        if self.setter.is_some() {
            let payload = PyProperty {
                getter: self.getter.clone(),
                setter: self.setter.clone(),
                deleter: None,
            };

            PyObject::new(payload, self.ctx.property_type())
        } else {
            let payload = PyReadOnlyProperty {
                getter: self.getter.expect(
                    "One of add_getter/add_setter must be called when constructing a property",
                ),
            };

            PyObject::new(payload, self.ctx.readonly_property_type())
        }
    }
}

pub fn init(context: &PyContext) {
    extend_class!(context, &context.readonly_property_type, {
        "__get__" => context.new_rustfunc(PyReadOnlyPropertyRef::get),
    });

    let property_doc =
        "Property attribute.\n\n  \
         fget\n    \
         function to be used for getting an attribute value\n  \
         fset\n    \
         function to be used for setting an attribute value\n  \
         fdel\n    \
         function to be used for del\'ing an attribute\n  \
         doc\n    \
         docstring\n\n\
         Typical use is to define a managed attribute x:\n\n\
         class C(object):\n    \
         def getx(self): return self._x\n    \
         def setx(self, value): self._x = value\n    \
         def delx(self): del self._x\n    \
         x = property(getx, setx, delx, \"I\'m the \'x\' property.\")\n\n\
         Decorators make defining new properties or modifying existing ones easy:\n\n\
         class C(object):\n    \
         @property\n    \
         def x(self):\n        \"I am the \'x\' property.\"\n        \
         return self._x\n    \
         @x.setter\n    \
         def x(self, value):\n        \
         self._x = value\n    \
         @x.deleter\n    \
         def x(self):\n        \
         del self._x";

    extend_class!(context, &context.property_type, {
        "__new__" => context.new_rustfunc(PyPropertyRef::new_property),
        "__doc__" => context.new_str(property_doc.to_string()),

        "__get__" => context.new_rustfunc(PyPropertyRef::get),
        "__set__" => context.new_rustfunc(PyPropertyRef::set),
        "__delete__" => context.new_rustfunc(PyPropertyRef::delete),

        "fget" => context.new_property(PyPropertyRef::fget),
        "fset" => context.new_property(PyPropertyRef::fset),
        "fdel" => context.new_property(PyPropertyRef::fdel),

        "getter" => context.new_rustfunc(PyPropertyRef::getter),
        "setter" => context.new_rustfunc(PyPropertyRef::setter),
        "deleter" => context.new_rustfunc(PyPropertyRef::deleter),
    });
}
