use core::ptr::NonNull;

use rustpython_common::lock::{PyMutex, PyRwLock};

use crate::{
    AsObject, PyObject, PyObjectRef, PyRef, PyStackRef, function::Either, object::PyObjectPayload,
};

pub type TraverseFn<'a> = dyn FnMut(&PyObject) + 'a;

/// This trait is used as a "Optional Trait"(I 'd like to use `Trace?` but it's not allowed yet) for PyObjectPayload type
///
/// impl for PyObjectPayload, `pyclass` proc macro will handle the actual dispatch if type impl `Trace`
/// Every PyObjectPayload impl `MaybeTrace`, which may or may not be traceable
pub trait MaybeTraverse {
    /// if is traceable, will be used by vtable to determine
    const HAS_TRAVERSE: bool = false;
    /// if has clear implementation for circular reference resolution (tp_clear)
    const HAS_CLEAR: bool = false;
    // if this type is traceable, then call with tracer_fn, default to do nothing
    fn try_traverse(&self, traverse_fn: &mut TraverseFn<'_>);
    // if this type has clear, extract child refs for circular reference resolution (tp_clear)
    fn try_clear(&mut self, _out: &mut Vec<PyObjectRef>) {}
}

/// Type that need traverse it's children should impl [`Traverse`] (not [`MaybeTraverse`])
/// # Safety
/// Please carefully read [`Traverse::traverse()`] and follow the guideline
pub unsafe trait Traverse {
    /// impl `traverse()` with caution! Following those guideline so traverse doesn't cause memory error!:
    /// - Make sure that every owned object(Every PyObjectRef/PyRef) is called with traverse_fn **at most once**.
    ///   If some field is not called, the worst results is just memory leak,
    ///   but if some field is called repeatedly, panic and deadlock can happen.
    ///
    /// - _**DO NOT**_ clone a [`PyObjectRef`] or [`PyRef<T>`] in [`Traverse::traverse()`]
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>);

    /// Extract all owned child PyObjectRefs for circular reference resolution (tp_clear).
    /// Called just before object deallocation to break circular references.
    /// Default implementation does nothing.
    fn clear(&mut self, _out: &mut Vec<PyObjectRef>) {}
}

unsafe impl Traverse for PyObjectRef {
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        traverse_fn(self)
    }
}

unsafe impl Traverse for PyStackRef {
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        traverse_fn(self.as_object())
    }
}

unsafe impl<T: PyObjectPayload> Traverse for PyRef<T> {
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        traverse_fn(self.as_object())
    }
}

unsafe impl Traverse for () {
    fn traverse(&self, _traverse_fn: &mut TraverseFn<'_>) {}
}

unsafe impl<T: Traverse> Traverse for Option<T> {
    #[inline]
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        if let Some(v) = self {
            v.traverse(traverse_fn);
        }
    }
}

unsafe impl<T> Traverse for [T]
where
    T: Traverse,
{
    #[inline]
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        for elem in self {
            elem.traverse(traverse_fn);
        }
    }
}

unsafe impl<T> Traverse for Box<[T]>
where
    T: Traverse,
{
    #[inline]
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        for elem in &**self {
            elem.traverse(traverse_fn);
        }
    }
}

unsafe impl<T> Traverse for Vec<T>
where
    T: Traverse,
{
    #[inline]
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        for elem in self {
            elem.traverse(traverse_fn);
        }
    }
}

unsafe impl<T: Traverse> Traverse for PyRwLock<T> {
    #[inline]
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        // A failed try_read means a writer holds the lock. Traversal runs with
        // the world stopped, but a thread force-parked while DETACHED (CAS'd
        // straight to SUSPENDED from native code) may still hold the write lock
        // it was in the middle of taking. Skipping such an object is safe: the
        // collector then does not see its outgoing edges, which only
        // under-traverses and thus over-approximates liveness (a conservative
        // keep-alive), never freeing a reachable object. In single-threaded
        // builds a failure only reflects the current thread's own re-entrant
        // read, likewise safely skipped.
        if let Some(inner) = self.try_read_recursive() {
            inner.traverse(traverse_fn)
        }
    }
}

/// Safety: the lock is not held across visiting children to avoid a re-entrant
/// deadlock. In threading builds traversal runs under stop-the-world so no
/// other thread mutates the guarded value while we read it; in single-threaded
/// builds there is no other writer.
unsafe impl<T: Traverse> Traverse for PyMutex<T> {
    #[inline]
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        let mut chs: Vec<NonNull<PyObject>> = Vec::new();
        if let Some(obj) = self.try_lock() {
            obj.traverse(&mut |ch| {
                chs.push(NonNull::from(ch));
            })
        }
        chs.iter()
            .map(|ch| {
                // Safety: the world is stopped (threading builds) or the
                // interpreter is single-threaded, so `ch` is not concurrently
                // freed while we hand it to the tracer.
                let ch = unsafe { ch.as_ref() };
                traverse_fn(ch);
            })
            .count();
    }
}

macro_rules! trace_tuple {
    ($(($NAME: ident, $NUM: tt)),*) => {
        unsafe impl<$($NAME: Traverse),*> Traverse for ($($NAME),*) {
            #[inline]
            fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
                $(
                    self.$NUM.traverse(traverse_fn);
                )*
            }
        }

    };
}

unsafe impl<A: Traverse, B: Traverse> Traverse for Either<A, B> {
    #[inline]
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        match self {
            Self::A(a) => a.traverse(tracer_fn),
            Self::B(b) => b.traverse(tracer_fn),
        }
    }
}

// only tuple with 12 elements or less is supported,
// because long tuple is extremely rare in almost every case
unsafe impl<A: Traverse> Traverse for (A,) {
    #[inline]
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.0.traverse(tracer_fn);
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
