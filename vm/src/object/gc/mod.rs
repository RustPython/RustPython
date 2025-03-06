use std::collections::HashSet;
use rustpython_common::lock::PyMutex;
use rustpython_common::rc::PyRc;
use crate::object::core::{Erased, PyInner};
use crate::object::Traverse;
use crate::PyObject;

/// A very basic tracing, stop-the-world garbage collector.
///
/// It maintains a list of allocated objects and, when triggered,
/// stops the world, marks all objects reachable from a set of root objects,
/// and sweeps away the rest.
pub struct GarbageCollector {
    /// All objects allocated on the GC heap.
    heap: Vec<*mut PyObject>,
    /// Set of objects reached during the mark phase.
    marked: HashSet<*mut PyObject>,
}

impl GarbageCollector {
    /// Create a new GC instance.
    pub fn new() -> Self {
        GarbageCollector {
            heap: Vec::new(),
            marked: HashSet::new(),
        }
    }

    /// Register a newly allocated object with the GC.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `obj` is a valid pointer to a PyObject.
    pub unsafe fn add_object(&mut self, obj: *mut PyObject) {
        self.heap.push(obj);
    }

    /// The mark phase: starting from the roots, mark all reachable objects.
    ///
    /// The `roots` slice should contain pointers to all the root objects.
    pub unsafe fn mark(&mut self, roots: &[*mut PyObject]) {
        for &root in roots {
            unsafe {
                self.mark_object(root);
            }
        }
    }

    /// Recursively mark an object and its children.
    ///
    /// If the object is null or already marked, the function returns immediately.
    unsafe fn mark_object(&mut self, obj: *mut PyObject) {
        if obj.is_null() || self.marked.contains(&obj) {
            return;
        }
        self.marked.insert(obj);

        // Define a tracer callback that recursively marks child objects.
        // We assume that `traverse` is implemented to call this callback
        // on each child.
        let mut tracer = |child: &PyObject| {
            // Safety: We assume that rust borrow checking rules are not violated.
            let child = child as *const PyObject as *mut PyObject;
            unsafe {
                self.mark_object(child);
            }
        };

        // Traverse the object’s children.
        // Safety: We assume that `obj` is a valid pointer with a properly implemented traverse.
        unsafe {
            (*obj).traverse(&mut tracer);
        }
    }

    /// The sweep phase: deallocate any object not marked as reachable.
    ///
    /// Unmarked objects are freed using `drop_dealloc_obj`. After sweeping,
    /// the `marked` set is cleared for the next GC cycle.
    pub unsafe fn sweep(&mut self) {
        self.heap.retain(|&obj| {
            if self.marked.contains(&obj) {
                // Object is reachable; keep it.
                true
            } else {
                // Object is unreachable; deallocate it.
                unsafe {
                    drop(unsafe { Box::from_raw(obj as *mut PyInner<Erased>) });
                }
                false
            }
        });
        self.marked.clear();
    }

    /// Perform a full garbage collection cycle.
    ///
    /// This stops the world, marks all objects reachable from `roots`,
    /// and then sweeps away the unmarked objects.
    ///
    /// # Safety
    ///
    /// The caller must ensure that no new allocations or mutations occur during GC.
    pub unsafe fn collect_garbage(&mut self) {
        unsafe {
            // TODO: Collect roots.
            self.mark(roots);
            self.sweep();
        }
    }
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "threading")]
pub static GLOBAL_COLLECTOR: once_cell::sync::Lazy<PyRc<PyMutex<GarbageCollector>>> =
    once_cell::sync::Lazy::new(|| PyRc::new(PyMutex::new(Default::default())));

#[cfg(not(feature = "threading"))]
thread_local! {
    pub static GLOBAL_COLLECTOR: PyRc<PyMutex<GarbageCollector>> = PyRc::new(PyMutex::new(Default::default()));
}

pub unsafe fn register_object(obj: *mut PyObject) {
    GLOBAL_COLLECTOR.lock().add_object(obj);
}

pub unsafe fn try_gc() {
    GLOBAL_COLLECTOR.lock().collect_garbage();
}
