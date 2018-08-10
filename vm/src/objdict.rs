use super::pyobject::{PyObjectRef, PyResult};
use super::vm::VirtualMachine;

pub fn _set_item(
    vm: &mut VirtualMachine,
    _d: PyObjectRef,
    _idx: PyObjectRef,
    _obj: PyObjectRef,
) -> PyResult {
    // TODO: Implement objdict::set_item
    Ok(vm.get_none())
}

/* TODO:
pub fn make_type() -> PyObjectRef {

    // dict.insert("__set_item__".to_string(), _set_item);
}
*/
