use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::c_int;
use rustpython_vm::function::{FuncArgs, KwArgs};

define_py_check!(exact fn PyTraceBack_Check, types.traceback_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceBack_Print(tb: *mut PyObject, file: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let tb = unsafe { &*tb };
        let file = unsafe { &*file };
        let tb_module = vm.import("traceback", 0)?;
        let print_tb = tb_module.get_attr("print_tb", vm)?;

        let kwargs: KwArgs = core::iter::once(("file".to_string(), file.to_owned())).collect();
        print_tb.call(FuncArgs::new(vec![tb.to_owned()], kwargs), vm)?;

        Ok(())
    })
}
