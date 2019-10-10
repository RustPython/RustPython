//! Builtin function specific to WASM build.
//!
//! This is required because some feature like I/O works differently in the browser comparing to
//! desktop.
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html.

use js_sys::{self, Array};
use web_sys::{self, console};

use rustpython_vm::function::PyFuncArgs;
use rustpython_vm::obj::{objstr, objtype};
use rustpython_vm::pyobject::{IdProtocol, PyResult, TypeProtocol};
use rustpython_vm::VirtualMachine;

pub(crate) fn window() -> web_sys::Window {
    web_sys::window().expect("Window to be available")
}

pub fn format_print_args(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult<String> {
    // Handle 'sep' kwarg:
    let sep_arg = args
        .get_optional_kwarg("sep")
        .filter(|obj| !obj.is(&vm.get_none()));
    if let Some(ref obj) = sep_arg {
        if !objtype::isinstance(obj, &vm.ctx.str_type()) {
            return Err(vm.new_type_error(format!(
                "sep must be None or a string, not {}",
                obj.class().name
            )));
        }
    }
    let sep_str = sep_arg.as_ref().map(|obj| objstr::borrow_value(obj));

    // Handle 'end' kwarg:
    let end_arg = args
        .get_optional_kwarg("end")
        .filter(|obj| !obj.is(&vm.get_none()));
    if let Some(ref obj) = end_arg {
        if !objtype::isinstance(obj, &vm.ctx.str_type()) {
            return Err(vm.new_type_error(format!(
                "end must be None or a string, not {}",
                obj.class().name
            )));
        }
    }
    let end_str = end_arg.as_ref().map(|obj| objstr::borrow_value(obj));

    // No need to handle 'flush' kwarg, irrelevant when writing to String

    let mut output = String::new();
    let mut first = true;
    for a in args.args {
        if first {
            first = false;
        } else if let Some(ref sep_str) = sep_str {
            output.push_str(sep_str);
        } else {
            output.push(' ');
        }
        output.push_str(&vm.to_pystr(&a)?);
    }

    if let Some(end_str) = end_str {
        output.push_str(end_str.as_ref())
    } else {
        output.push('\n');
    }
    Ok(output)
}

pub fn builtin_print_console(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    let arr = Array::new();
    for arg in args.args {
        arr.push(&vm.to_pystr(&arg)?.into());
    }
    console::log(&arr);
    Ok(vm.get_none())
}
