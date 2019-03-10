use std::fmt;

use crate::pyobject::{PyContext, PyNativeFunc, PyObjectPayload2, PyObjectRef};

pub struct PyBuiltinFunction {
    // TODO: shouldn't be public
    pub value: PyNativeFunc,
}

impl PyObjectPayload2 for PyBuiltinFunction {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.builtin_function_or_method_type()
    }
}

impl fmt::Debug for PyBuiltinFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "builtin function")
    }
}

impl PyBuiltinFunction {
    pub fn new(value: PyNativeFunc) -> Self {
        Self { value }
    }
}
