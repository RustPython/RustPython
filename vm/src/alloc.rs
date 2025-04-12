use std::alloc::{Layout, GlobalAlloc};
use std::cell::UnsafeCell;
use std::ffi::CStr;

// Pretend these are the actual allocator impls

type GA_Alloc = unsafe fn(layout: Layout) -> *mut u8;
type GA_Dealloc = unsafe fn(ptr: *mut u8, layout: Layout);
type GA_AllocZeroed = unsafe fn(layout: Layout) -> *mut u8;
type GA_Realloc = unsafe fn(ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8;

mod mimalloc_functions {
    use std::alloc::Layout;

    use mimalloc::MiMalloc;

    pub unsafe fn alloc(layout: Layout) -> *mut u8 {
        MiMalloc::alloc(layout)
    }
    
    pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
        MiMalloc::dealloc(ptr, layout)
    }
    
    pub unsafe fn alloc_zeroed(layout: Layout) -> *mut u8 {
        MiMalloc::alloc_zeroed(layout)
    }
    
    pub unsafe fn realloc(ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        MiMalloc::realloc(ptr, layout, new_size)
    }
}

mod malloc_functions {
    use std::alloc::System as Malloc;
    use std::alloc::Layout;

    pub unsafe fn alloc(layout: Layout) -> *mut u8 {
        Malloc::alloc(layout)
    }
    
    pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
        Malloc::dealloc(ptr, layout)
    }
    
    pub unsafe fn alloc_zeroed(layout: Layout) -> *mut u8 {
        Malloc::alloc_zeroed(layout)
    }
    
    pub unsafe fn realloc(ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        Malloc::realloc(ptr, layout, new_size)
    }
}

struct ConfigurableAllocator {
    alloc: GA_Alloc,
    dealloc: GA_Dealloc,
    alloc_zeroed: GA_AllocZeroed,
    realloc: GA_Realloc,
}

struct MakeMutable<T> {
    inner: UnsafeCell<T>,
}

impl<T> MakeMutable<T> {
    const fn new(inner: T) -> Self {
        MakeMutable {
            inner: UnsafeCell::new(inner),
        }
    }

    fn get(&self) -> &T {
        unsafe { &*self.inner.get() }
    }

    fn get_mut(&self) -> &mut T {
        unsafe { &mut *self.inner.get() }
    }
}

unsafe impl<T: Sync> Sync for MakeMutable<T> {}

unsafe impl GlobalAlloc for MakeMutable<ConfigurableAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ((*self.inner.get()).alloc)(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ((*self.inner.get()).dealloc)(ptr, layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ((*self.inner.get()).alloc_zeroed)(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ((*self.inner.get()).realloc)(ptr, layout, new_size)
    }
}

impl ConfigurableAllocator {
    const fn default() -> Self {
        ConfigurableAllocator {
            alloc: mimalloc_functions::alloc,
            dealloc: mimalloc_functions::dealloc,
            alloc_zeroed: mimalloc_functions::alloc_zeroed,
            realloc: mimalloc_functions::realloc,
        }
    }
}

#[global_allocator]
static ALLOC: MakeMutable<ConfigurableAllocator> = MakeMutable::new(ConfigurableAllocator::default());

fn switch_to_malloc() {
    unsafe {
        (*ALLOC.inner.get()).alloc = malloc_functions::alloc;
        (*ALLOC.inner.get()).dealloc = malloc_functions::dealloc;
        (*ALLOC.inner.get()).alloc_zeroed = malloc_functions::alloc_zeroed;
        (*ALLOC.inner.get()).realloc = malloc_functions::realloc;
    }
}

fn switch_to_mimalloc() {
    unsafe {
        (*ALLOC.inner.get()).alloc = mimalloc_functions::alloc;
        (*ALLOC.inner.get()).dealloc = mimalloc_functions::dealloc;
        (*ALLOC.inner.get()).alloc_zeroed = mimalloc_functions::alloc_zeroed;
        (*ALLOC.inner.get()).realloc = mimalloc_functions::realloc;
    }
}
