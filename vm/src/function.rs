use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;

use crate::obj::objtype;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    IntoPyObject, PyContext, PyObject, PyObjectPayload, PyObjectPayload2, PyObjectRef, PyResult,
    TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

// TODO: Move PyFuncArgs, FromArgs, etc. here

// TODO: `PyRef` probably actually belongs in the pyobject module.

/// A reference to the payload of a built-in object.
///
/// Note that a `PyRef<T>` can only deref to a shared / immutable reference.
/// It is the payload type's responsibility to handle (possibly concurrent)
/// mutability with locks or concurrent data structures if required.
///
/// A `PyRef<T>` can be directly returned from a built-in function to handle
/// situations (such as when implementing in-place methods such as `__iadd__`)
/// where a reference to the same object must be returned.
#[derive(Clone)]
pub struct PyRef<T> {
    // invariant: this obj must always have payload of type T
    obj: PyObjectRef,
    _payload: PhantomData<T>,
}

impl<T> PyRef<T>
where
    T: PyObjectPayload2,
{
    pub fn new(ctx: &PyContext, payload: T) -> Self {
        PyRef {
            obj: PyObject::new(
                PyObjectPayload::AnyRustValue {
                    value: Box::new(payload),
                },
                T::required_type(ctx),
            ),
            _payload: PhantomData,
        }
    }

    pub fn new_with_type(vm: &mut VirtualMachine, payload: T, cls: PyClassRef) -> PyResult<Self> {
        let required_type = T::required_type(&vm.ctx);
        if objtype::issubclass(&cls.obj, &required_type) {
            Ok(PyRef {
                obj: PyObject::new(
                    PyObjectPayload::AnyRustValue {
                        value: Box::new(payload),
                    },
                    cls.obj,
                ),
                _payload: PhantomData,
            })
        } else {
            let subtype = vm.to_pystr(&cls.obj)?;
            let basetype = vm.to_pystr(&required_type)?;
            Err(vm.new_type_error(format!("{} is not a subtype of {}", subtype, basetype)))
        }
    }

    pub fn as_object(&self) -> &PyObjectRef {
        &self.obj
    }
    pub fn into_object(self) -> PyObjectRef {
        self.obj
    }
}

impl<T> Deref for PyRef<T>
where
    T: PyObjectPayload2,
{
    type Target = T;

    fn deref(&self) -> &T {
        self.obj.payload().expect("unexpected payload for type")
    }
}

impl<T> TryFromObject for PyRef<T>
where
    T: PyObjectPayload2,
{
    fn try_from_object(vm: &mut VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if objtype::isinstance(&obj, &T::required_type(&vm.ctx)) {
            Ok(PyRef {
                obj,
                _payload: PhantomData,
            })
        } else {
            let expected_type = vm.to_pystr(&T::required_type(&vm.ctx))?;
            let actual_type = vm.to_pystr(&obj.typ())?;
            Err(vm.new_type_error(format!(
                "Expected type {}, not {}",
                expected_type, actual_type,
            )))
        }
    }
}

impl<T> IntoPyObject for PyRef<T> {
    fn into_pyobject(self, _ctx: &PyContext) -> PyResult {
        Ok(self.obj)
    }
}

impl<T> fmt::Display for PyRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.obj.fmt(f)
    }
}
