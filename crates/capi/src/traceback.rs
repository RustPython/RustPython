use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::c_int;
use rustpython_vm::function::{FuncArgs, KwArgs};

#[unsafe(no_mangle)]
pub extern "C" fn PyTraceBack_Print(tb: *mut PyObject, file: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let tb = unsafe { &*tb };
        let file = unsafe { &*file };
        let tb_module = vm.import("traceback", 0)?;
        let print_tb = tb_module.get_attr("print_tb", vm)?;

        let kwargs: KwArgs = [("file".to_string(), file.to_owned())]
            .into_iter()
            .collect();
        print_tb.call(FuncArgs::new(vec![tb.to_owned()], kwargs), vm)?;

        Ok(())
    })
}
