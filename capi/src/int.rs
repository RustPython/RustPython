use rustpython_vm::{PyObject, PyObjectRef};

#[unsafe(export_name = "PyLong_FromLong")]
pub unsafe extern "C" fn long_from_long(value: i64) -> *mut PyObject {
    let vm = crate::get_vm();
    Into::<PyObjectRef>::into(vm.ctx.new_int(value))
        .into_raw()
        .as_ptr()
}

#[unsafe(export_name = "PyLong_FromUnsignedLong")]
pub unsafe extern "C" fn long_from_unsigned_long(value: u64) -> *mut PyObject {
    let vm = crate::get_vm();
    Into::<PyObjectRef>::into(vm.ctx.new_int(value))
        .into_raw()
        .as_ptr()
}
