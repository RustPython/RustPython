use std::ptr::NonNull;

use rustpython_common::lock::{PyMutex, PyRwLock};

use crate::{function::Either, object::PyObjectPayload, AsObject, PyObject, PyObjectRef, PyRef};

pub type TracerFn<'a> = dyn FnMut(&PyObject) + 'a;

/// This trait is used as a "Optional Trait"(I 'd like to use `Trace?` but it's not allowed yet) for PyObjectPayload type
///
/// impl for PyObjectPayload, `pyclass` proc macro will handle the actual dispatch if type impl `Trace`
/// Every PyObjectPayload impl `MaybeTrace`, which may or may not be traceable
pub trait MaybeTrace {
    /// if is traceable, will be used by vtable to determine
    const IS_TRACE: bool = false;
    // if this type is traceable, then call with tracer_fn, default to do nothing
    fn try_trace(&self, tracer_fn: &mut TracerFn);
}

/// Type that need trace it's children should impl `Trace`(Not `MaybeTrace`)
/// # Safety
/// impl `trace()` with caution! Following those guideline so trace doesn't cause memory error!:
/// - Make sure that every owned object(Every PyObjectRef/PyRef) is called with tracer_fn **at most once**.
/// If some field is not called, the worst results is just memory leak,
/// but if some field is called repeatly, panic and deadlock can happen.
///
/// - _**DO NOT**_ clone a `PyObjectRef` or `Pyef<T>` in `trace()`
pub unsafe trait Trace {
    /// impl `trace()` with caution! Following those guideline so trace doesn't cause memory error!:
    /// - Make sure that every owned object(Every PyObjectRef/PyRef) is called with tracer_fn **at most once**.
    /// If some field is not called, the worst results is just memory leak,
    /// but if some field is called repeatly, panic and deadlock can happen.
    ///
    /// - _**DO NOT**_ clone a `PyObjectRef` or `Pyef<T>` in `trace()`
    fn trace(&self, tracer_fn: &mut TracerFn);
}

unsafe impl Trace for PyObjectRef {
    fn trace(&self, tracer_fn: &mut TracerFn) {
        tracer_fn(self)
    }
}

unsafe impl<T: PyObjectPayload> Trace for PyRef<T> {
    fn trace(&self, tracer_fn: &mut TracerFn) {
        tracer_fn(self.as_object())
    }
}

unsafe impl Trace for () {
    fn trace(&self, _tracer_fn: &mut TracerFn) {}
}

unsafe impl<T: Trace> Trace for Option<T> {
    #[inline]
    fn trace(&self, tracer_fn: &mut TracerFn) {
        if let Some(v) = self {
            v.trace(tracer_fn);
        }
    }
}

unsafe impl<T> Trace for [T]
where
    T: Trace,
{
    #[inline]
    fn trace(&self, tracer_fn: &mut TracerFn) {
        for elem in self {
            elem.trace(tracer_fn);
        }
    }
}

unsafe impl<T> Trace for Box<[T]>
where
    T: Trace,
{
    #[inline]
    fn trace(&self, tracer_fn: &mut TracerFn) {
        for elem in &**self {
            elem.trace(tracer_fn);
        }
    }
}

unsafe impl<T> Trace for Vec<T>
where
    T: Trace,
{
    #[inline]
    fn trace(&self, tracer_fn: &mut TracerFn) {
        for elem in self {
            elem.trace(tracer_fn);
        }
    }
}

unsafe impl<T: Trace> Trace for PyRwLock<T> {
    #[inline]
    fn trace(&self, tracer_fn: &mut TracerFn) {
        // if can't get a lock, this means something else is holding the lock,
        // but since gc stopped the world, during gc the lock is always held
        // so it is safe to ignore those in gc
        if let Some(inner) = self.try_read_recursive() {
            inner.trace(tracer_fn)
        }
    }
}

/// Safety: We can't hold lock during traverse it's child because it may cause deadlock.
/// TODO(discord9): check if this is thread-safe to do
/// (Outside of gc phase, only incref/decref will call trace,
/// and refcnt is atomic, so it should be fine?)
unsafe impl<T: Trace> Trace for PyMutex<T> {
    #[inline]
    fn trace(&self, tracer_fn: &mut TracerFn) {
        let mut chs: Vec<NonNull<PyObject>> = Vec::new();
        if let Some(obj) = self.try_lock() {
            obj.trace(&mut |ch| {
                chs.push(NonNull::from(ch));
            })
        }
        chs.iter()
            .map(|ch| {
                // Safety: during gc, this should be fine, because nothing should write during gc's tracing?
                let ch = unsafe { ch.as_ref() };
                tracer_fn(ch);
            })
            .count();
    }
}

macro_rules! trace_tuple {
    ($(($NAME: ident, $NUM: tt)),*) => {
        unsafe impl<$($NAME: Trace),*> Trace for ($($NAME),*) {
            #[inline]
            fn trace(&self, tracer_fn: &mut TracerFn) {
                $(
                    self.$NUM.trace(tracer_fn);
                )*
            }
        }

    };
}

unsafe impl<A: Trace, B: Trace> Trace for Either<A, B> {
    #[inline]
    fn trace(&self, tracer_fn: &mut TracerFn) {
        match self {
            Either::A(a) => a.trace(tracer_fn),
            Either::B(b) => b.trace(tracer_fn),
        }
    }
}

// only tuple with 12 elements or less is supported,
// because long tuple is extremly rare in almost every case
unsafe impl<A: Trace> Trace for (A,) {
    #[inline]
    fn trace(&self, tracer_fn: &mut TracerFn) {
        self.0.trace(tracer_fn);
    }
}
trace_tuple!((A, 0), (B, 1));
trace_tuple!((A, 0), (B, 1), (C, 2));
trace_tuple!((A, 0), (B, 1), (C, 2), (D, 3));
trace_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4));
trace_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5));
trace_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6));
trace_tuple!(
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7)
);
trace_tuple!(
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7),
    (I, 8)
);
trace_tuple!(
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7),
    (I, 8),
    (J, 9)
);
trace_tuple!(
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7),
    (I, 8),
    (J, 9),
    (K, 10)
);
trace_tuple!(
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7),
    (I, 8),
    (J, 9),
    (K, 10),
    (L, 11)
);
