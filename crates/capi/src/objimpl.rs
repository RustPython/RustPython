use crate::PyObject;
use crate::pymem::{PyMem_Calloc, PyMem_Free, PyMem_Malloc, PyMem_Realloc};
use crate::pystate::with_vm;
use core::ffi::{c_int, c_void};
use rustpython_vm::gc_state;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_Track(op: *mut PyObject) {
    with_vm(|_vm| {
        let obj = unsafe { &*op };
        if !obj.is_gc_tracked() {
            unsafe { gc_state::gc_state().track_object(obj.into()) };
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_UnTrack(op: *mut PyObject) {
    with_vm(|_vm| {
        let obj = unsafe { &*op };
        if obj.is_gc_tracked() {
            unsafe { gc_state::gc_state().untrack_object(obj.into()) };
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_IsTracked(op: *mut PyObject) -> c_int {
    with_vm(|_vm| unsafe { (&*op).is_gc_tracked() })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GC_IsFinalized(op: *mut PyObject) -> c_int {
    with_vm(|_vm| unsafe { (&*op).gc_finalized() })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGC_Collect() -> isize {
    let result = gc_state::gc_state().collect(2);
    (result.collected + result.uncollectable) as isize
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGC_Enable() -> c_int {
    let gc = gc_state::gc_state();
    let was_enabled = gc.is_enabled();
    gc.enable();
    was_enabled.into()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGC_Disable() -> c_int {
    let gc = gc_state::gc_state();
    let was_enabled = gc.is_enabled();
    gc.disable();
    was_enabled.into()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGC_IsEnabled() -> c_int {
    gc_state::gc_state().is_enabled().into()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Malloc(size: usize) -> *mut c_void {
    unsafe { PyMem_Malloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Calloc(nelem: usize, elsize: usize) -> *mut c_void {
    unsafe { PyMem_Calloc(nelem, elsize) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Realloc(ptr: *mut c_void, new_size: usize) -> *mut c_void {
    unsafe { PyMem_Realloc(ptr, new_size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Free(ptr: *mut c_void) {
    unsafe { PyMem_Free(ptr) }
}
