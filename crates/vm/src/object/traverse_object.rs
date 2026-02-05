use alloc::fmt;
use core::any::TypeId;

use crate::{
    PyObject, PyObjectRef,
    object::{
        Erased, InstanceDict, MaybeTraverse, PyInner, PyObjectPayload, debug_obj, default_dealloc,
        try_clear_obj, try_traverse_obj,
    },
};

use super::{Traverse, TraverseFn};

pub(in crate::object) struct PyObjVTable {
    pub(in crate::object) typeid: TypeId,
    /// dealloc: handles __del__, weakref clearing, and memory free.
    pub(in crate::object) dealloc: unsafe fn(*mut PyObject),
    pub(in crate::object) debug: unsafe fn(&PyObject, &mut fmt::Formatter<'_>) -> fmt::Result,
    pub(in crate::object) trace: Option<unsafe fn(&PyObject, &mut TraverseFn<'_>)>,
    /// Clear for circular reference resolution (tp_clear).
    /// Called just before deallocation to extract child references.
    pub(in crate::object) clear: Option<unsafe fn(*mut PyObject, &mut Vec<PyObjectRef>)>,
}

impl PyObjVTable {
    pub const fn of<T: PyObjectPayload>() -> &'static Self {
        &Self {
            typeid: T::PAYLOAD_TYPE_ID,
            dealloc: default_dealloc::<T>,
            debug: debug_obj::<T>,
            trace: const {
                if T::HAS_TRAVERSE {
                    Some(try_traverse_obj::<T>)
                } else {
                    None
                }
            },
            clear: const {
                if T::HAS_CLEAR {
                    Some(try_clear_obj::<T>)
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
        // For heap type instances, traverse the type reference.
        // PyAtomicRef holds a strong reference (via PyRef::leak), so GC must
        // account for it to correctly detect instance ↔ type cycles.
        // Static types are always alive and don't need this.
        let typ = &*self.typ;
        if typ.heaptype_ext.is_some() {
            // Safety: Py<PyType> and PyObject share the same memory layout
            let typ_obj: &PyObject = unsafe { &*(typ as *const _ as *const PyObject) };
            tracer_fn(typ_obj);
        }
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
        // For heap type instances, traverse the type reference (same as erased version)
        let typ = &*self.typ;
        if typ.heaptype_ext.is_some() {
            let typ_obj: &PyObject = unsafe { &*(typ as *const _ as *const PyObject) };
            tracer_fn(typ_obj);
        }
        self.dict.traverse(tracer_fn);
        // weak_list is inline atomic pointers, no heap allocation, no trace
        self.slots.traverse(tracer_fn);
        T::try_traverse(&self.payload, tracer_fn);
    }
}
