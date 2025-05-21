use std::fmt;

use crate::PyObject;

use super::core::{InstanceDict, debug_obj, drop_dealloc_obj, try_trace_obj};
use super::{PyObjectPayload, Traverse, TraverseFn};

pub(in crate::object) struct PyObjVTable {
    pub(in crate::object) drop_dealloc: unsafe fn(*mut PyObject),
    pub(in crate::object) debug: unsafe fn(&PyObject, &mut fmt::Formatter<'_>) -> fmt::Result,
    pub(in crate::object) trace: Option<unsafe fn(&PyObject, &mut TraverseFn<'_>)>,
}

impl PyObjVTable {
    pub const fn of<T: PyObjectPayload>() -> &'static Self {
        &PyObjVTable {
            drop_dealloc: drop_dealloc_obj::<T>,
            debug: debug_obj::<T>,
            trace: const {
                if T::IS_TRACE {
                    Some(try_trace_obj::<T>)
                } else {
                    None
                }
            },
        }
    }
}

unsafe impl Traverse for InstanceDict {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.d.traverse(tracer_fn)
    }
}
