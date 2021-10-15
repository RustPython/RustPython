use std::borrow::{Borrow, Cow};

use crate::{IdProtocol, PyObjectRef, PyResult, TypeProtocol, VirtualMachine};

// Sequence Protocol
// https://docs.python.org/3/c-api/sequence.html

#[allow(clippy::type_complexity)]
#[derive(Default, Clone)]
pub struct PySequenceMethods {
    pub length: Option<fn(&PyObjectRef, &VirtualMachine) -> PyResult<usize>>,
    pub concat: Option<fn(&PyObjectRef, &PyObjectRef, &VirtualMachine) -> PyResult<PyObjectRef>>,
    pub repeat: Option<fn(&PyObjectRef, usize, &VirtualMachine) -> PyResult<PyObjectRef>>,
    pub inplace_concat:
        Option<fn(PyObjectRef, &PyObjectRef, &VirtualMachine) -> PyResult<PyObjectRef>>,
    pub inplace_repeat: Option<fn(PyObjectRef, usize, &VirtualMachine) -> PyResult<PyObjectRef>>,
    pub item: Option<fn(&PyObjectRef, isize, &VirtualMachine) -> PyResult<PyObjectRef>>,
    pub ass_item:
        Option<fn(PyObjectRef, isize, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
    pub contains: Option<fn(&PyObjectRef, &PyObjectRef, &VirtualMachine) -> PyResult<bool>>,
}

pub struct PySequence(PyObjectRef, Cow<'static, PySequenceMethods>);

impl PySequence {
    pub fn check(obj: &PyObjectRef, vm: &VirtualMachine) -> bool {
        let cls = obj.class();
        if cls.is(&vm.ctx.types.dict_type) {
            return false;
        }
        if let Some(f) = cls.mro_find_map(|x| x.slots.as_sequence.load()) {
            return f(obj, vm).item.is_some();
        }
        false
    }

    pub fn from_object(vm: &VirtualMachine, obj: PyObjectRef) -> Option<Self> {
        let cls = obj.class();
        if cls.is(&vm.ctx.types.dict_type) {
            return None;
        }
        let f = cls.mro_find_map(|x| x.slots.as_sequence.load())?;
        drop(cls);
        let methods = f(&obj, vm);
        if methods.item.is_some() {
            Some(Self(obj, methods))
        } else {
            None
        }
    }

    pub fn methods(&self) -> &PySequenceMethods {
        self.1.borrow()
    }
}
