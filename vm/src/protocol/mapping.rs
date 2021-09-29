//! Mapping protocol

use crate::{vm::VirtualMachine, PyObjectRef, PyResult, TryFromBorrowedObject, TypeProtocol};

#[allow(clippy::type_complexity)]
pub struct PyMapping {
    pub length: Option<fn(PyObjectRef, &VirtualMachine) -> PyResult<usize>>,
    pub subscript: Option<fn(PyObjectRef, PyObjectRef, &VirtualMachine) -> PyResult>,
    pub ass_subscript:
        Option<fn(PyObjectRef, PyObjectRef, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
}

impl PyMapping {
    pub fn check(cls: &PyObjectRef, vm: &VirtualMachine) -> bool {
        if let Ok(mapping) = PyMapping::try_from_borrowed_object(vm, cls) {
            mapping.subscript.is_some()
        } else {
            false
        }
    }
}

impl TryFromBorrowedObject for PyMapping {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
        let obj_cls = obj.class();
        for cls in obj_cls.iter_mro() {
            if let Some(f) = cls.slots.as_mapping.load() {
                return f(obj, vm);
            }
        }
        Err(vm.new_type_error(format!(
            "a dict-like object is required, not '{}'",
            obj_cls.name()
        )))
    }
}
