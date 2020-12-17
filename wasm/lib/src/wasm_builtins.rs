//! Builtin function specific to WASM build.
//!
//! This is required because some feature like I/O works differently in the browser comparing to
//! desktop.
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html.

use web_sys::{self, console};

use rustpython_vm::builtins::PyStrRef;
use rustpython_vm::pyobject::{BorrowValue, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;

pub(crate) fn window() -> web_sys::Window {
    web_sys::window().expect("Window to be available")
}

pub fn sys_stdout_write_console(data: &str, _vm: &VirtualMachine) -> PyResult<()> {
    console::log_1(&data.into());
    Ok(())
}

pub fn make_stdout_object(
    vm: &VirtualMachine,
    write_f: impl Fn(&str, &VirtualMachine) -> PyResult<()> + 'static,
) -> PyObjectRef {
    let ctx = &vm.ctx;
    let write_method = ctx.new_method(
        "write",
        move |_self: PyObjectRef, data: PyStrRef, vm: &VirtualMachine| -> PyResult<()> {
            write_f(data.borrow_value(), vm)
        },
    );
    let flush_method = ctx.new_method("flush", |_self: PyObjectRef| {});
    // there's not really any point to storing this class so that there's a consistent type object,
    // we just want a half-decent repr() output
    let cls = py_class!(ctx, "JSStdout", &vm.ctx.types.object_type, {
        "write" => write_method,
        "flush" => flush_method,
    });
    ctx.new_base_object(cls, None)
}
