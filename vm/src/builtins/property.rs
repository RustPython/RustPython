/*! Python `property` descriptor class.

*/
use crate::common::lock::PyRwLock;

use super::pytype::PyTypeRef;
use crate::function::FuncArgs;
use crate::pyobject::{
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
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
    getter: PyRwLock<Option<PyObjectRef>>,
    setter: PyRwLock<Option<PyObjectRef>>,
    deleter: PyRwLock<Option<PyObjectRef>>,
    doc: PyRwLock<Option<PyObjectRef>>,
}

impl PyValue for PyProperty {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.property_type
    }
}

#[derive(FromArgs)]
struct PropertyArgs {
    #[pyarg(any, default)]
    fget: Option<PyObjectRef>,
    #[pyarg(any, default)]
    fset: Option<PyObjectRef>,
    #[pyarg(any, default)]
    fdel: Option<PyObjectRef>,
    #[pyarg(any, default)]
    doc: Option<PyObjectRef>,
}

impl SlotDescriptor for PyProperty {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(zelf, obj, vm)?;
        if vm.is_none(&obj) {
            Ok(zelf.into_object())
        } else if let Some(getter) = zelf.getter.read().as_ref() {
            vm.invoke(&getter, (obj,))
        } else {
            Err(vm.new_attribute_error("unreadable attribute".to_string()))
        }
    }
}

#[pyimpl(with(SlotDescriptor), flags(BASETYPE))]
impl PyProperty {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyProperty {
            getter: PyRwLock::new(None),
            setter: PyRwLock::new(None),
            deleter: PyRwLock::new(None),
            doc: PyRwLock::new(None),
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn init(&self, args: PropertyArgs) {
        *self.getter.write() = args.fget;
        *self.setter.write() = args.fset;
        *self.deleter.write() = args.fdel;
        *self.doc.write() = args.doc;
    }

    // Descriptor methods

    #[pyslot]
    fn descr_set(
        zelf: PyObjectRef,
        obj: PyObjectRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let zelf = PyRef::<Self>::try_from_object(vm, zelf)?;
        match value {
            Some(value) => {
                if let Some(ref setter) = zelf.setter.read().as_ref() {
                    vm.invoke(setter, vec![obj, value]).map(drop)
                } else {
                    Err(vm.new_attribute_error("can't set attribute".to_owned()))
                }
            }
            None => {
                if let Some(ref deleter) = zelf.deleter.read().as_ref() {
                    vm.invoke(deleter, (obj,)).map(drop)
                } else {
                    Err(vm.new_attribute_error("can't delete attribute".to_owned()))
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
        Self::descr_set(zelf, obj, Some(value), vm)
    }
    #[pymethod]
    fn __delete__(zelf: PyObjectRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::descr_set(zelf, obj, None, vm)
    }

    // Access functions

    #[pyproperty]
    fn fget(&self) -> Option<PyObjectRef> {
        self.getter.read().clone()
    }

    #[pyproperty]
    fn fset(&self) -> Option<PyObjectRef> {
        self.setter.read().clone()
    }

    #[pyproperty]
    fn fdel(&self) -> Option<PyObjectRef> {
        self.deleter.read().clone()
    }

    fn doc_getter(&self) -> Option<PyObjectRef> {
        self.doc.read().clone()
    }
    fn doc_setter(&self, value: Option<PyObjectRef>) {
        *self.doc.write() = value;
    }

    // Python builder functions

    #[pymethod]
    fn getter(
        zelf: PyRef<Self>,
        getter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        PyProperty {
            getter: PyRwLock::new(getter.or_else(|| zelf.fget())),
            setter: PyRwLock::new(zelf.fset()),
            deleter: PyRwLock::new(zelf.fdel()),
            doc: PyRwLock::new(None),
        }
        .into_ref_with_type(vm, TypeProtocol::clone_class(&zelf))
    }

    #[pymethod]
    fn setter(
        zelf: PyRef<Self>,
        setter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        PyProperty {
            getter: PyRwLock::new(zelf.fget()),
            setter: PyRwLock::new(setter.or_else(|| zelf.fset())),
            deleter: PyRwLock::new(zelf.fdel()),
            doc: PyRwLock::new(None),
        }
        .into_ref_with_type(vm, TypeProtocol::clone_class(&zelf))
    }

    #[pymethod]
    fn deleter(
        zelf: PyRef<Self>,
        deleter: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        PyProperty {
            getter: PyRwLock::new(zelf.fget()),
            setter: PyRwLock::new(zelf.fset()),
            deleter: PyRwLock::new(deleter.or_else(|| zelf.fdel())),
            doc: PyRwLock::new(None),
        }
        .into_ref_with_type(vm, TypeProtocol::clone_class(&zelf))
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
