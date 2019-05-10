use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "itertools", {
    })
}
