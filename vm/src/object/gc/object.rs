#[cfg(debug_assertions)]
use super::super::ID2TYPE;
use super::super::{
    core::{debug_obj, PyInner},
    ext::AsObject,
    payload::PyObjectPayload,
    Erased, InstanceDict, Py, PyObject, PyRef,
};
use super::{GcHeader, GcStatus, Trace, TracerFn};
use std::{fmt, marker::PhantomData, ptr::NonNull};

pub(in crate::object) struct PyObjVTable {
    pub(in crate::object) drop_dealloc: unsafe fn(*mut PyObject),
    pub(in crate::object) drop_only: unsafe fn(*mut PyObject),
    pub(in crate::object) dealloc_only: unsafe fn(*mut PyObject),
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
                drop_only: drop_only_obj::<T>,
                dealloc_only: dealloc_only_obj::<T>,
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

unsafe fn drop_dealloc_obj<T: PyObjectPayload>(x: *mut PyObject) {
    if (*x).header().buffered() {
        error!("Try to drop&dealloc a buffered object! Drop only for now!");
        drop_only_obj::<T>(x);
    } else {
        drop(Box::from_raw(x as *mut PyInner<T>));
    }
}

macro_rules! partially_drop {
    ($OBJ: ident. $($(#[$attr:meta])? $FIELD: ident),*) => {
        $(
            $(#[$attr])?
            NonNull::from(&$OBJ.$FIELD).as_ptr().drop_in_place();
        )*
    };
}

/// drop only(doesn't deallocate)
/// NOTE: `header` is not drop to prevent UB
unsafe fn drop_only_obj<T: PyObjectPayload>(x: *mut PyObject) {
    let obj = &*x.cast::<PyInner<T>>();
    partially_drop!(
        obj.
        #[cfg(debug_assertions)]
        is_drop,
        typeid,
        typ,
        dict,
        slots,
        payload
    );
}

unsafe impl Trace for PyInner<Erased> {
    fn trace(&self, tracer_fn: &mut TracerFn) {
        // trace PyInner's other field(that is except payload)
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

unsafe impl<T: PyObjectPayload> Trace for Py<T> {
    fn trace(&self, tracer_fn: &mut TracerFn) {
        self.as_object().0.trace(tracer_fn)
    }
}

unsafe impl Trace for PyObject {
    fn trace(&self, tracer_fn: &mut TracerFn) {
        self.0.trace(tracer_fn)
    }
}

/// deallocate memory with type info(cast as PyInner<T>) in heap only, DOES NOT run destructor
/// # Safety
/// - should only be called after its' destructor is done(i.e. called `drop_value`(which called drop_in_place))
/// - panic on a null pointer
/// move drop `header` here to prevent UB
unsafe fn dealloc_only_obj<T: PyObjectPayload>(x: *mut PyObject) {
    {
        let obj = &*x.cast::<PyInner<T>>();
        partially_drop!(obj.header, vtable, weak_list);
    } // don't want keep a ref to a to be deallocated object
    std::alloc::dealloc(
        x.cast(),
        std::alloc::Layout::for_value(&*x.cast::<PyInner<T>>()),
    );
}

unsafe fn try_trace_obj<T: PyObjectPayload>(x: &PyObject, tracer_fn: &mut TracerFn) {
    let x = &*(x as *const PyObject as *const PyInner<T>);
    let payload = &x.payload;
    payload.try_trace(tracer_fn)
}

unsafe impl Trace for InstanceDict {
    fn trace(&self, tracer_fn: &mut TracerFn) {
        self.d.trace(tracer_fn)
    }
}

impl PyObject {
    pub(in crate::object) fn header(&self) -> &GcHeader {
        &self.0.header
    }

    pub(in crate::object) fn is_traceable(&self) -> bool {
        self.0.vtable.trace.is_some()
    }
    pub(in crate::object) fn increment(&self) {
        self.0.header.gc().increment(self)
    }
    pub(in crate::object) fn decrement(&self) -> GcStatus {
        self.0.header.gc().decrement(self)
    }
    /// only clear weakref and then run rust RAII destructor, no `__del__` neither dealloc
    pub(in crate::object) unsafe fn drop_clr_wr(ptr: NonNull<PyObject>) -> bool {
        #[cfg(feature = "gc_bacon")]
        if !ptr.as_ref().header().check_set_drop_only() {
            return false;
        }
        let zelf = ptr.as_ref();
        zelf.clear_weakref();

        // not set PyInner's is_drop because still havn't dealloc
        let drop_only = zelf.0.vtable.drop_only;

        drop_only(ptr.as_ptr());
        // Safety: after drop_only, header should still remain undropped
        #[cfg(feature = "gc_bacon")]
        ptr.as_ref().header().set_done_drop(true);
        true
    }

    /// run object's __del__ and then rust's destructor but doesn't dealloc
    pub(in crate::object) unsafe fn del_drop(ptr: NonNull<PyObject>) -> bool {
        if let Err(()) = ptr.as_ref().try_del() {
            // abort drop for whatever reason
            return false;
        }

        Self::drop_clr_wr(ptr)
    }
    /// call `drop_only` in vtable
    pub(in crate::object) unsafe fn drop_only(ptr: NonNull<PyObject>) {
        let zelf = ptr.as_ref();
        // not set PyInner's is_drop because still havn't dealloc
        let drop_only = zelf.0.vtable.drop_only;

        drop_only(ptr.as_ptr());
    }
    pub(in crate::object) unsafe fn dealloc_only(ptr: NonNull<PyObject>) -> bool {
        // can't check for if is a alive PyWeak here because already dropped payload
        #[cfg(feature = "gc_bacon")]
        {
            if !ptr.as_ref().header().check_set_dealloc_only() {
                return false;
            }
        }

        #[cfg(debug_assertions)]
        {
            *ptr.as_ref().0.is_drop.lock() = true;
        }
        let dealloc_only = ptr.as_ref().0.vtable.dealloc_only;

        dealloc_only(ptr.as_ptr());
        true
    }
}

impl<T: PyObjectPayload> Drop for PyRef<T> {
    #[inline]
    fn drop(&mut self) {
        let _no_gc = self.0.header.try_pausing();
        #[cfg(debug_assertions)]
        {
            if *self.0.is_drop.lock() {
                error!(
                    "Double drop on PyRef<{}>",
                    std::any::type_name::<T>().to_string()
                );
                return;
            }
            let tid = std::any::TypeId::of::<T>();
            ID2TYPE
                .lock()
                .expect("can't insert into ID2TYPE")
                .entry(tid)
                .or_insert_with(|| std::any::type_name::<T>().to_string());
        }
        let stat = self.as_object().decrement();
        let ptr = self.ptr.cast::<PyObject>();
        match stat {
            GcStatus::ShouldDrop => unsafe {
                PyObject::drop_slow(ptr);
            },
            GcStatus::BufferedDrop => unsafe {
                PyObject::del_drop(ptr);
            },
            GcStatus::GarbageCycle => unsafe {
                PyObject::del_only(ptr);
            },
            GcStatus::ShouldKeep | GcStatus::DoNothing => (),
        }
    }
}
