//! Builtin function specific to WASM build.
//!
//! This is required because some feature like I/O works differently in the browser comparing to
//! desktop.
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html.

use js_sys::{self, Array};
use web_sys::{self, console};

use rustpython_vm::function::{Args, KwArgs, PyFuncArgs};
use rustpython_vm::import;
use rustpython_vm::obj::{
    objstr::{self, PyStringRef},
    objtype,
};
use rustpython_vm::pyobject::{IdProtocol, ItemProtocol, PyObjectRef, PyResult, TypeProtocol};
use rustpython_vm::VirtualMachine;

pub(crate) fn window() -> web_sys::Window {
    web_sys::window().expect("Window to be available")
}

pub fn format_print_args(vm: &VirtualMachine, args: PyFuncArgs) -> Result<String, PyObjectRef> {
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

pub fn builtin_import(
    module_name: PyStringRef,
    _args: Args,
    _kwargs: KwArgs,
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    let module_name = module_name.as_str();

    let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules").unwrap();

    // First, see if we already loaded the module:
    if let Ok(module) = sys_modules.get_item(module_name.to_string(), vm) {
        Ok(module)
    } else if vm.frozen.borrow().contains_key(module_name) {
        import::import_frozen(vm, module_name)
    } else if vm.stdlib_inits.borrow().contains_key(module_name) {
        import::import_builtin(vm, module_name)
    } else {
        let notfound_error = vm.context().exceptions.module_not_found_error.clone();
        Err(vm.new_exception(
            notfound_error,
            format!("Module {:?} not found", module_name),
        ))
    }
}
