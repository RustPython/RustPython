/// Thread data stack for interpreter frames (`_PyStackChunk` /
/// `tstate->datastack_*`).
///
/// A linked list of chunks providing bump allocation for frame-local data
/// (localsplus arrays).  Normal function calls allocate via `push()`
/// (pointer bump).  Generators and coroutines use heap-allocated storage.
use alloc::alloc::{alloc, dealloc};
use core::alloc::Layout;
use core::ptr;

/// Minimum chunk size in bytes (`_PY_DATA_STACK_CHUNK_SIZE`).
const MIN_CHUNK_SIZE: usize = 16 * 1024;

/// Extra headroom (in bytes) to avoid allocating a new chunk for the next
/// frame right after growing.
const MINIMUM_OVERHEAD: usize = 1000 * core::mem::size_of::<usize>();

/// Alignment for all data stack allocations.
const ALIGN: usize = 16;

/// Header for a data stack chunk.  The usable data region starts right after
/// this header (aligned to `ALIGN`).
#[repr(C)]
struct DataStackChunk {
    /// Previous chunk in the linked list (NULL for the root chunk).
    previous: *mut DataStackChunk,
    /// Total allocation size in bytes (including this header).
    size: usize,
    /// Saved `top` offset when a newer chunk was pushed.  Used to restore
    /// `DataStack::top` when popping back to this chunk.
    saved_top: usize,
}

impl DataStackChunk {
    /// Pointer to the first usable byte after the header (aligned).
    #[inline(always)]
    fn data_start(&self) -> *mut u8 {
        let header_end = (self as *const Self as usize) + core::mem::size_of::<Self>();
        let aligned = (header_end + ALIGN - 1) & !(ALIGN - 1);
        aligned as *mut u8
    }

    /// Pointer past the last usable byte.
    #[inline(always)]
    fn data_limit(&self) -> *mut u8 {
        unsafe { (self as *const Self as *mut u8).add(self.size) }
    }
}

/// Per-thread data stack for bump-allocating frame-local data.
pub struct DataStack {
    /// Current chunk.
    chunk: *mut DataStackChunk,
    /// Current allocation position within the current chunk.
    top: *mut u8,
    /// End of usable space in the current chunk.
    limit: *mut u8,
}

impl DataStack {
    /// Create a new data stack with an initial root chunk.
    #[must_use]
    pub fn new() -> Self {
        let chunk = Self::alloc_chunk(MIN_CHUNK_SIZE, ptr::null_mut());
        let top = unsafe { (*chunk).data_start() };
        let limit = unsafe { (*chunk).data_limit() };
        // Skip one ALIGN-sized slot in the root chunk so that `pop()` never
        // frees it (`push_chunk` convention).
        let top = unsafe { top.add(ALIGN) };
        Self { chunk, top, limit }
    }

    /// Check if the current chunk has at least `size` bytes available.
    #[inline(always)]
    #[must_use]
    pub fn has_space(&self, size: usize) -> bool {
        let aligned_size = (size + ALIGN - 1) & !(ALIGN - 1);
        (self.limit as usize).saturating_sub(self.top as usize) >= aligned_size
    }

    /// Allocate `size` bytes from the data stack.
    ///
    /// Returns a pointer to the allocated region (aligned to `ALIGN`).
    /// The caller must call `pop()` with the returned pointer when done
    /// (LIFO order).
    #[inline(always)]
    pub fn push(&mut self, size: usize) -> *mut u8 {
        let aligned_size = (size + ALIGN - 1) & !(ALIGN - 1);
        unsafe {
            if self.top.add(aligned_size) <= self.limit {
                let ptr = self.top;
                self.top = self.top.add(aligned_size);
                ptr
            } else {
                self.push_slow(aligned_size)
            }
        }
    }

    /// Slow path: allocate a new chunk and push from it.
    #[cold]
    #[inline(never)]
    fn push_slow(&mut self, aligned_size: usize) -> *mut u8 {
        let mut chunk_size = MIN_CHUNK_SIZE;
        let needed = aligned_size
            .checked_add(MINIMUM_OVERHEAD)
            .and_then(|v| v.checked_add(core::mem::size_of::<DataStackChunk>()))
            .and_then(|v| v.checked_add(ALIGN))
            .expect("DataStack chunk size overflow");
        while chunk_size < needed {
            chunk_size = chunk_size
                .checked_mul(2)
                .expect("DataStack chunk size overflow");
        }
        // Save current position in old chunk.
        unsafe {
            (*self.chunk).saved_top = self.top as usize - self.chunk as usize;
        }
        let new_chunk = Self::alloc_chunk(chunk_size, self.chunk);
        self.chunk = new_chunk;
        let start = unsafe { (*new_chunk).data_start() };
        self.limit = unsafe { (*new_chunk).data_limit() };
        self.top = unsafe { start.add(aligned_size) };
        start
    }

    /// Pop a previous allocation.  `base` must be the pointer returned by
    /// `push()`.  Calls must be in LIFO order.
    ///
    /// # Safety
    /// `base` must be a valid pointer returned by `push()` on this data stack,
    /// and all allocations made after it must already have been popped.
    #[inline(always)]
    pub unsafe fn pop(&mut self, base: *mut u8) {
        debug_assert!(!base.is_null());
        if self.is_in_current_chunk(base) {
            // Common case: base is within the current chunk.
            self.top = base;
        } else {
            // base is in a previous chunk — free the current chunk.
            unsafe { self.pop_slow(base) };
        }
    }

    /// Check if `ptr` falls within the current chunk's data area.
    /// Both bounds are checked to handle non-monotonic allocation addresses
    /// (e.g. on Windows where newer chunks may be at lower addresses).
    #[inline(always)]
    fn is_in_current_chunk(&self, ptr: *mut u8) -> bool {
        let chunk_start = unsafe { (*self.chunk).data_start() };
        ptr >= chunk_start && ptr <= self.limit
    }

    /// Slow path: pop back to a previous chunk.
    #[cold]
    #[inline(never)]
    unsafe fn pop_slow(&mut self, base: *mut u8) {
        loop {
            let old_chunk = self.chunk;
            let prev = unsafe { (*old_chunk).previous };
            debug_assert!(!prev.is_null(), "tried to pop past the root chunk");
            unsafe { Self::free_chunk(old_chunk) };
            self.chunk = prev;
            self.limit = unsafe { (*prev).data_limit() };
            if self.is_in_current_chunk(base) {
                self.top = base;
                return;
            }
        }
    }

    /// Allocate a new chunk.
    fn alloc_chunk(size: usize, previous: *mut DataStackChunk) -> *mut DataStackChunk {
        let layout = Layout::from_size_align(size, ALIGN).expect("invalid chunk layout");
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            alloc::alloc::handle_alloc_error(layout);
        }
        let chunk = ptr as *mut DataStackChunk;
        unsafe {
            (*chunk).previous = previous;
            (*chunk).size = size;
            (*chunk).saved_top = 0;
        }
        chunk
    }

    /// Free a chunk.
    unsafe fn free_chunk(chunk: *mut DataStackChunk) {
        let size = unsafe { (*chunk).size };
        let layout = Layout::from_size_align(size, ALIGN).expect("invalid chunk layout");
        unsafe { dealloc(chunk as *mut u8, layout) };
    }
}

// SAFETY: DataStack is per-thread and not shared.  The raw pointers
// it contains point to memory exclusively owned by this DataStack.
unsafe impl Send for DataStack {}

impl Default for DataStack {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DataStack {
    fn drop(&mut self) {
        let mut chunk = self.chunk;
        while !chunk.is_null() {
            let prev = unsafe { (*chunk).previous };
            unsafe { Self::free_chunk(chunk) };
            chunk = prev;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_push_pop() {
        let mut ds = DataStack::new();
        let p1 = ds.push(64);
        assert!(!p1.is_null());
        let p2 = ds.push(128);
        assert!(!p2.is_null());
        assert!(p2 > p1);
        unsafe {
            ds.pop(p2);
            ds.pop(p1);
        }
    }

    #[test]
    fn cross_chunk_push_pop() {
        let mut ds = DataStack::new();
        // Push enough to force a new chunk
        let mut ptrs = Vec::new();
        for _ in 0..100 {
            ptrs.push(ds.push(1024));
        }
        // Pop all in reverse
        for p in ptrs.into_iter().rev() {
            unsafe { ds.pop(p) };
        }
    }

    #[test]
    fn alignment() {
        let mut ds = DataStack::new();
        for size in [1, 7, 15, 16, 17, 31, 32, 33, 64, 100] {
            let p = ds.push(size);
            assert_eq!(p as usize % ALIGN, 0, "alignment violated for size {size}");
            unsafe { ds.pop(p) };
        }
    }
}
