//! Opaque instance body owned by C.
//!
//! The bytes are zero-filled storage for a foreign type. Dropping the payload
//! frees that storage and does not read it.

use crate::builtins::PyType;
use crate::object::{MaybeTraverse, TraverseFn};
use crate::vm::Context;
use crate::{Py, PyPayload};
use alloc::alloc::{alloc_zeroed, dealloc};
use core::alloc::Layout;
use core::fmt;

const ALIGN: usize = 16;

/// Instance payload whose body is a C-owned byte buffer of `basicsize` bytes.
pub struct PyCBody {
    ptr: *mut u8,
    size: usize,
}

impl PyCBody {
    /// Allocate `size` zeroed bytes, aligned to 16. `size` must be non-zero.
    #[must_use]
    pub fn new(size: usize) -> Option<Self> {
        let layout = Layout::from_size_align(size, ALIGN).ok()?;
        if layout.size() == 0 {
            return None;
        }
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            return None;
        }
        Some(Self { ptr, size })
    }

    #[must_use]
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.ptr
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for PyCBody {
    fn drop(&mut self) {
        let Ok(layout) = Layout::from_size_align(self.size, ALIGN) else {
            return;
        };
        if layout.size() == 0 || self.ptr.is_null() {
            return;
        }
        unsafe { dealloc(self.ptr, layout) };
    }
}

impl fmt::Debug for PyCBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PyCBody").field("size", &self.size).finish()
    }
}

// The buffer is owned by this value. Sharing the pointer across threads is the
// same contract as any other payload: callers mutate it only on the attached thread.
unsafe impl Send for PyCBody {}
unsafe impl Sync for PyCBody {}

impl MaybeTraverse for PyCBody {
    fn try_traverse(&self, _traverse_fn: &mut TraverseFn<'_>) {}
}

impl PyPayload for PyCBody {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.object_type
    }
}
