use crate::pyobject::PyObjectRef;
use crate::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "_json", {})
}
