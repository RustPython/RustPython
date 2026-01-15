use core::ptr::NonNull;

use rustpython_common::lock::{PyMutex, PyRwLock};

use crate::{AsObject, PyObject, PyObjectRef, PyRef, function::Either, object::PyObjectPayload};

pub type TraverseFn<'a> = dyn FnMut(&PyObject) + 'a;

/// This trait is used as a "Optional Trait"(I 'd like to use `Trace?` but it's not allowed yet) for PyObjectPayload type
///
/// impl for PyObjectPayload, `pyclass` proc macro will handle the actual dispatch if type impl `Trace`
/// Every PyObjectPayload impl `MaybeTrace`, which may or may not be traceable
pub trait MaybeTraverse {
    /// if is traceable, will be used by vtable to determine
    const IS_TRACE: bool = false;
    // if this type is traceable, then call with tracer_fn, default to do nothing
    fn try_traverse(&self, traverse_fn: &mut TraverseFn<'_>);
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
}

unsafe impl Traverse for PyObjectRef {
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        traverse_fn(self)
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
        // if can't get a lock, this means something else is holding the lock,
        // but since gc stopped the world, during gc the lock is always held
        // so it is safe to ignore those in gc
        if let Some(inner) = self.try_read_recursive() {
            inner.traverse(traverse_fn)
        }
    }
}

/// Safety: We can't hold lock during traverse it's child because it may cause deadlock.
/// TODO(discord9): check if this is thread-safe to do
/// (Outside of gc phase, only incref/decref will call trace,
/// and refcnt is atomic, so it should be fine?)
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
                // Safety: during gc, this should be fine, because nothing should write during gc's tracing?
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
