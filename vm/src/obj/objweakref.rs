use super::objtype::PyClassRef;
use crate::function::OptionalArg;
use crate::pyobject::{
    PyContext, PyObject, PyObjectPayload, PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::vm::VirtualMachine;

use std::rc::{Rc, Weak};

#[derive(Debug)]
pub struct PyWeak {
    referent: Weak<PyObject<dyn PyObjectPayload>>,
}

impl PyWeak {
    pub fn downgrade(obj: &PyObjectRef) -> PyWeak {
        PyWeak {
            referent: Rc::downgrade(obj),
        }
    }

    pub fn upgrade(&self) -> Option<PyObjectRef> {
        self.referent.upgrade()
    }
}

impl PyValue for PyWeak {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.weakref_type()
    }
}

pub type PyWeakRef = PyRef<PyWeak>;

impl PyWeakRef {
    // TODO callbacks
    fn create(
        cls: PyClassRef,
        referent: PyObjectRef,
        _callback: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        PyWeak::downgrade(&referent).into_ref_with_type(vm, cls)
    }

    fn call(self, vm: &VirtualMachine) -> PyObjectRef {
        self.referent.upgrade().unwrap_or_else(|| vm.get_none())
    }
}

pub fn init(context: &PyContext) {
    extend_class!(context, &context.types.weakref_type, {
        (slot new) => PyWeakRef::create,
        "__call__" => context.new_rustfunc(PyWeakRef::call)
    });
}
