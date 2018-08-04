
use super::pyobject::{PyObjectKind, PyObjectRef, PyResult};

pub fn boolval(o: PyObjectRef) -> bool {
    let obj = o.borrow();
    match obj.kind {
        PyObjectKind::Boolean { value } => value,
        PyObjectKind::Integer { value } => value != 0,
        ref kind => unimplemented!("converting to boolean unsupported for: {:?}", kind),
    }
}
