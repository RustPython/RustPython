use std::marker::PhantomData;
use std::ops::Deref;

use crate::obj::objtype;
use crate::pyobject::{PyObjectPayload2, PyObjectRef, PyResult, TryFromObject};
use crate::vm::VirtualMachine;

// TODO: Move PyFuncArgs, FromArgs, etc. here

pub struct PyRef<T> {
    // invariant: this obj must always have payload of type T
    obj: PyObjectRef,
    _payload: PhantomData<T>,
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
            Err(vm.new_type_error("wrong type".to_string())) // TODO: better message
        }
    }
}
