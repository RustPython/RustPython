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
            unsafe { gc_state::gc_state().track_object(obj.into(), gc_state::current_owner()) };
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
    with_vm(|vm| {
        let result = vm.state.gc.collect(2);
        (result.collected + result.uncollectable) as isize
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGC_Enable() -> c_int {
    with_vm(|vm| {
        let was_enabled: c_int = vm.state.gc.is_enabled().into();
        vm.state.gc.enable();
        was_enabled
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGC_Disable() -> c_int {
    with_vm(|vm| {
        let was_enabled: c_int = vm.state.gc.is_enabled().into();
        vm.state.gc.disable();
        was_enabled
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGC_IsEnabled() -> c_int {
    with_vm(|vm| -> c_int { vm.state.gc.is_enabled().into() })
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
