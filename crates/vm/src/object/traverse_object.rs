use alloc::fmt;
use core::any::TypeId;

use crate::{
    PyObject,
    object::{
        Erased, InstanceDict, MaybeTraverse, PyInner, PyObjectPayload, debug_obj, drop_dealloc_obj,
        try_traverse_obj,
    },
};

use super::{Traverse, TraverseFn};

pub(in crate::object) struct PyObjVTable {
    pub(in crate::object) typeid: TypeId,
    pub(in crate::object) drop_dealloc: unsafe fn(*mut PyObject),
    pub(in crate::object) debug: unsafe fn(&PyObject, &mut fmt::Formatter<'_>) -> fmt::Result,
    pub(in crate::object) trace: Option<unsafe fn(&PyObject, &mut TraverseFn<'_>)>,
}

impl PyObjVTable {
    pub const fn of<T: PyObjectPayload>() -> &'static Self {
        &Self {
            typeid: T::PAYLOAD_TYPE_ID,
            drop_dealloc: drop_dealloc_obj::<T>,
            debug: debug_obj::<T>,
            trace: const {
                if T::HAS_TRAVERSE {
                    Some(try_traverse_obj::<T>)
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

unsafe impl Traverse for PyInner<Erased> {
    /// Because PyObject hold a `PyInner<Erased>`, so we need to trace it
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        // 1. trace `dict` and `slots` field(`typ` can't trace for it's a AtomicRef while is leaked by design)
        // 2. call vtable's trace function to trace payload
        // self.typ.trace(tracer_fn);
        self.dict.traverse(tracer_fn);
        // weak_list is inline atomic pointers, no heap allocation, no trace
        self.slots.traverse(tracer_fn);

        if let Some(f) = self.vtable.trace {
            unsafe {
                let zelf = &*(self as *const Self as *const PyObject);
                f(zelf, tracer_fn)
            }
        };
    }
}

unsafe impl<T: MaybeTraverse> Traverse for PyInner<T> {
    /// Type is known, so we can call `try_trace` directly instead of using erased type vtable
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        // 1. trace `dict` and `slots` field(`typ` can't trace for it's a AtomicRef while is leaked by design)
        // 2. call corresponding `try_trace` function to trace payload
        // (No need to call vtable's trace function because we already know the type)
        // self.typ.trace(tracer_fn);
        self.dict.traverse(tracer_fn);
        // weak_list is inline atomic pointers, no heap allocation, no trace
        self.slots.traverse(tracer_fn);
        T::try_traverse(&self.payload, tracer_fn);
    }
}
