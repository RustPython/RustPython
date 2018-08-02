use super::pyobject::{PyObjectRef, PyResult};
use super::vm::VirtualMachine;

pub fn set_item(vm: &mut VirtualMachine, d: PyObjectRef, idx: PyObjectRef, obj: PyObjectRef) -> PyResult {
    Ok(vm.get_none())
}

/* TODO:
pub fn make_type() -> PyObjectRef {

    // dict.insert("__set_item__".to_string(), set_item);
}
*/
