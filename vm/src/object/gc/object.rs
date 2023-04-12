use std::{fmt, marker::PhantomData};

use crate::{
    object::{
        debug_obj, drop_dealloc_obj, try_trace_obj, Erased, InstanceDict, PyInner, PyObjectPayload,
    },
    PyObject,
};

use super::{Trace, TracerFn};

pub(in crate::object) struct PyObjVTable {
    pub(in crate::object) drop_dealloc: unsafe fn(*mut PyObject),
    pub(in crate::object) debug: unsafe fn(&PyObject, &mut fmt::Formatter) -> fmt::Result,
    pub(in crate::object) trace: Option<unsafe fn(&PyObject, &mut TracerFn)>,
}

impl PyObjVTable {
    pub fn of<T: PyObjectPayload>() -> &'static Self {
        struct Helper<T: PyObjectPayload>(PhantomData<T>);
        trait VtableHelper {
            const VTABLE: PyObjVTable;
        }
        impl<T: PyObjectPayload> VtableHelper for Helper<T> {
            const VTABLE: PyObjVTable = PyObjVTable {
                drop_dealloc: drop_dealloc_obj::<T>,
                debug: debug_obj::<T>,
                trace: {
                    if T::IS_TRACE {
                        Some(try_trace_obj::<T>)
                    } else {
                        None
                    }
                },
            };
        }
        &Helper::<T>::VTABLE
    }
}

unsafe impl Trace for InstanceDict {
    fn trace(&self, tracer_fn: &mut TracerFn) {
        self.d.trace(tracer_fn)
    }
}

unsafe impl Trace for PyInner<Erased> {
    /// Because PyObject hold a `PyInner<Erased>`, so we need to trace it
    fn trace(&self, tracer_fn: &mut TracerFn) {
        // 1. trace `dict` and `slots` field(`typ` can't trace for it's a AtomicRef while is leaked by design)
        // 2. call vtable's trace function to trace payload
        // self.typ.trace(tracer_fn);
        self.dict.trace(tracer_fn);
        // weak_list keeps a *pointer* to a struct for maintaince weak ref, so no ownership, no trace
        self.slots.trace(tracer_fn);

        if let Some(f) = self.vtable.trace {
            unsafe {
                let zelf = &*(self as *const PyInner<Erased> as *const PyObject);
                f(zelf, tracer_fn)
            }
        };
    }
}

unsafe impl<T: PyObjectPayload> Trace for PyInner<T> {
    /// Type is known, so we can call `try_trace` directly instead of using erased type vtable
    fn trace(&self, tracer_fn: &mut TracerFn) {
        // 1. trace `dict` and `slots` field(`typ` can't trace for it's a AtomicRef while is leaked by design)
        // 2. call corrsponding `try_trace` function to trace payload
        // (No need to call vtable's trace function because we already know the type)
        // self.typ.trace(tracer_fn);
        self.dict.trace(tracer_fn);
        // weak_list keeps a *pointer* to a struct for maintaince weak ref, so no ownership, no trace
        self.slots.trace(tracer_fn);
        T::try_trace(&self.payload, tracer_fn);
    }
}
