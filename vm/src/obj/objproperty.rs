/*! Python `property` descriptor class.

*/
use crate::common::cell::PyRwLock;

use super::objtype::PyClassRef;
use crate::function::OptionalArg;
use crate::pyobject::{
    IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::slots::SlotDescriptor;
use crate::vm::VirtualMachine;

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
#[pyclass(module = false, name = "property")]
#[derive(Debug)]
pub struct PyProperty {
    getter: Option<PyObjectRef>,
    setter: Option<PyObjectRef>,
    deleter: Option<PyObjectRef>,
    doc: PyRwLock<Option<PyObjectRef>>,
}

impl PyValue for PyProperty {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.property_type.clone()
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

impl SlotDescriptor for PyProperty {
    fn descr_get(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: OptionalArg<PyObjectRef>,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(zelf, obj, vm)?;
        if vm.is_none(&obj) {
            Ok(zelf.into_object())
        } else if let Some(getter) = zelf.getter.as_ref() {
            vm.invoke(&getter, obj)
        } else {
            Err(vm.new_attribute_error("unreadable attribute".to_string()))
        }
    }
}

#[pyimpl(with(SlotDescriptor), flags(BASETYPE))]
impl PyProperty {
    #[pyslot]
    fn tp_new(cls: PyClassRef, args: PropertyArgs, vm: &VirtualMachine) -> PyResult<PyPropertyRef> {
        PyProperty {
            getter: args.fget,
            setter: args.fset,
            deleter: args.fdel,
            doc: PyRwLock::new(args.doc),
        }
        .into_ref_with_type(vm, cls)
    }

    // Descriptor methods

    #[pymethod(name = "__set__")]
    fn set(&self, obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(ref setter) = self.setter.as_ref() {
            vm.invoke(setter, vec![obj, value])
        } else {
            Err(vm.new_attribute_error("can't set attribute".to_owned()))
        }
    }

    #[pymethod(name = "__delete__")]
    fn delete(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(ref deleter) = self.deleter.as_ref() {
            vm.invoke(deleter, obj)
        } else {
            Err(vm.new_attribute_error("can't delete attribute".to_owned()))
        }
    }

    // Access functions

    #[pyproperty]
    fn fget(&self) -> Option<PyObjectRef> {
        self.getter.clone()
    }

    #[pyproperty]
    fn fset(&self) -> Option<PyObjectRef> {
        self.setter.clone()
    }

    #[pyproperty]
    fn fdel(&self) -> Option<PyObjectRef> {
        self.deleter.clone()
    }

    fn doc_getter(&self) -> Option<PyObjectRef> {
        self.doc.read().clone()
    }

    fn doc_setter(&self, value: PyObjectRef, vm: &VirtualMachine) {
        *self.doc.write() = py_none_to_option(vm, &value);
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
            doc: PyRwLock::new(None),
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
            doc: PyRwLock::new(None),
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
            doc: PyRwLock::new(None),
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

pub(crate) fn init(context: &PyContext) {
    PyProperty::extend_class(context, &context.types.property_type);

    // This is a bit unfortunate, but this instance attribute overlaps with the
    // class __doc__ string..
    extend_class!(context, &context.types.property_type, {
        "__doc__" => context.new_getset("__doc__", PyProperty::doc_getter, PyProperty::doc_setter),
    });
}
