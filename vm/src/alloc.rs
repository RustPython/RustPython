//! Configurable allocator for RustPython.
//! Currently it supports `mimalloc` and `system` allocators,
//! whereas cpython uses `pymalloc`` for most operations.

#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(any(target_env = "musl", target_env = "sgx"))
))]
mod inner {
    use std::alloc::{GlobalAlloc, Layout, System};

    pub enum RustPythonAllocator {
        System(System),
        Mimalloc(mimalloc::MiMalloc),
    }

    impl RustPythonAllocator {
        pub fn new(allocator: &str) -> Self {
            match allocator {
                "system" | "malloc" => RustPythonAllocator::System(System),
                "pymalloc" | "mimalloc" | "default" => {
                    RustPythonAllocator::Mimalloc(mimalloc::MiMalloc)
                }
                _ => RustPythonAllocator::System(System),
            }
        }
    }

    unsafe impl GlobalAlloc for RustPythonAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe {
                match self {
                    RustPythonAllocator::System(system) => system.alloc(layout),
                    RustPythonAllocator::Mimalloc(mimalloc) => mimalloc.alloc(layout),
                }
            }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe {
                match self {
                    RustPythonAllocator::System(system) => system.dealloc(ptr, layout),
                    RustPythonAllocator::Mimalloc(mimalloc) => mimalloc.dealloc(ptr, layout),
                }
            }
        }
    }
}

#[cfg(not(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(any(target_env = "musl", target_env = "sgx"))
)))]
mod inner {
    use std::alloc::{GlobalAlloc, Layout, System};

    pub enum RustPythonAllocator {
        System(System),
    }

    impl RustPythonAllocator {
        pub fn new(_allocator: &str) -> Self {
            RustPythonAllocator::System(System)
        }
    }

    impl GlobalAlloc for RustPythonAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe { self.0.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { self.0.dealloc(ptr, layout) }
        }
    }
}

use std::alloc::{GlobalAlloc, Layout};
use std::cell::UnsafeCell;

pub use inner::RustPythonAllocator as InternalAllocator;

pub struct RustPythonAllocator {
    inner: UnsafeCell<InternalAllocator>,
}

unsafe impl Send for RustPythonAllocator {}
unsafe impl Sync for RustPythonAllocator {}

impl RustPythonAllocator {
    /// Create a new allocator based on the PYTHONMALLOC environment variable
    /// or the default allocator if not set.
    /// If this is not intended, use [`InternalAllocator::new`] directly.
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(None),
        }
    }
}

impl RustPythonAllocator {
    unsafe fn get_or_init(&self) -> &InternalAllocator {
        unsafe {
            let inner = self.inner.get();
            if *inner.is_none() {
                let env = std::env::var("PYTHONMALLOC").unwrap_or_default(); 
                let allocator = InternalAllocator::new(&env);
                *inner = allocator;
            }
            inner as *const InternalAllocator as _
        }
    }
}

unsafe impl GlobalAlloc for RustPythonAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            let inner = self.get_or_init();
            inner.alloc(layout)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            let inner = self.get_or_init();
            inner.dealloc(ptr, layout)
        }
    }
}
