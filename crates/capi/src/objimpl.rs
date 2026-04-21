use crate::PyObject;
use crate::pystate::with_vm;
use crate::handles::resolve_object_handle;
use rustpython_vm::gc_state;

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GC_UnTrack(op: *mut PyObject) {
    with_vm(|_vm| {
        let obj = unsafe { &*resolve_object_handle(op) };
        if obj.is_gc_tracked() {
            unsafe { gc_state::gc_state().untrack_object(obj.into()) };
        }
    })
}
