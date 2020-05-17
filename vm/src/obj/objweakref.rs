use super::objtype::PyClassRef;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    PyClassImpl, PyContext, PyObject, PyObjectPayload, PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::slots::SlotCall;
use crate::vm::VirtualMachine;

use std::sync::{Arc, Weak};

#[pyclass]
#[derive(Debug)]
pub struct PyWeak {
    referent: Weak<PyObject<dyn PyObjectPayload>>,
}

impl PyWeak {
    pub fn downgrade(obj: &PyObjectRef) -> PyWeak {
        PyWeak {
            referent: Arc::downgrade(obj),
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

impl SlotCall for PyWeak {
    fn call(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        args.bind::<()>(vm)?;
        Ok(self.referent.upgrade().unwrap_or_else(|| vm.get_none()))
    }
}

#[pyimpl(with(SlotCall), flags(BASETYPE))]
impl PyWeak {
    // TODO callbacks
    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        referent: PyObjectRef,
        _callback: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        PyWeak::downgrade(&referent).into_ref_with_type(vm, cls)
    }
}

pub fn init(context: &PyContext) {
    PyWeak::extend_class(context, &context.types.weakref_type);
}
