use super::pyobject::{Executor, PyObjectRef, PyResult};

pub fn set_item(rt: &mut Executor, d: PyObjectRef, idx: PyObjectRef, obj: PyObjectRef) -> PyResult {
    Ok(rt.get_none())
}

/* TODO:
pub fn make_type() -> PyObjectRef {

    // dict.insert("__set_item__".to_string(), set_item);
}
*/
