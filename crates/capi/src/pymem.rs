use core::ffi::c_void;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Malloc(n: usize) -> *mut c_void {
    unsafe { libc::malloc(if n == 0 { 1 } else { n }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Calloc(nelem: usize, elsize: usize) -> *mut c_void {
    unsafe {
        libc::calloc(
            if nelem == 0 || elsize == 0 { 1 } else { nelem },
            if nelem == 0 || elsize == 0 { 1 } else { elsize },
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Realloc(ptr: *mut c_void, new_size: usize) -> *mut c_void {
    unsafe { libc::realloc(ptr, if new_size == 0 { 1 } else { new_size }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_Free(ptr: *mut c_void) {
    unsafe { libc::free(ptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawMalloc(n: usize) -> *mut c_void {
    unsafe { libc::malloc(n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawCalloc(nelem: usize, elsize: usize) -> *mut c_void {
    unsafe { libc::calloc(nelem, elsize) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawRealloc(ptr: *mut c_void, new_size: usize) -> *mut c_void {
    unsafe { libc::realloc(ptr, new_size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMem_RawFree(ptr: *mut c_void) {
    unsafe { libc::free(ptr) }
}
