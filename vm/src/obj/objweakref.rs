use crate::obj::objtype::PyClassRef;
use crate::pyobject::PyValue;
use crate::pyobject::{PyContext, PyObject, PyObjectRef, PyRef, PyResult};
use crate::vm::VirtualMachine;

use std::rc::{Rc, Weak};

#[derive(Debug)]
pub struct PyWeak {
    referent: Weak<PyObject>,
}

impl PyWeak {
    pub fn downgrade(obj: PyObjectRef) -> PyWeak {
        PyWeak {
            referent: Rc::downgrade(&obj),
        }
    }

    pub fn upgrade(&self) -> Option<PyObjectRef> {
        self.referent.upgrade()
    }
}

impl PyValue for PyWeak {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.weakref_type()
    }
}

pub type PyWeakRef = PyRef<PyWeak>;

impl PyWeakRef {
    // TODO callbacks
    fn create(cls: PyClassRef, referent: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<Self> {
        Self::new_with_type(vm, PyWeak::downgrade(referent), cls)
    }

    fn call(self, vm: &mut VirtualMachine) -> PyObjectRef {
        self.referent.upgrade().unwrap_or_else(|| vm.get_none())
    }
}

pub fn init(context: &PyContext) {
    extend_class!(context, &context.weakref_type, {
        "__new__" => context.new_rustfunc(PyWeakRef::create),
        "__call__" => context.new_rustfunc(PyWeakRef::call)
    });
}
