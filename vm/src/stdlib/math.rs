/* Math builtin module
 *
 *
 */

use super::super::obj::{objfloat, objtype};
use super::super::pyobject::{
    DictProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::VirtualMachine;

// Trigonometric functions:
fn math_acos(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let value = objfloat::get_value(value);
    let value = value.acos();
    let value = vm.ctx.new_float(value);
    Ok(value)
}

fn math_asin(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let value = objfloat::get_value(value);
    let value = value.asin();
    let value = vm.ctx.new_float(value);
    Ok(value)
}

fn math_atan(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let value = objfloat::get_value(value);
    let value = value.atan();
    let value = vm.ctx.new_float(value);
    Ok(value)
}

fn math_cos(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let value = objfloat::get_value(value);
    let value = value.cos();
    let value = vm.ctx.new_float(value);
    Ok(value)
}

fn math_sin(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let value = objfloat::get_value(value);
    let value = value.sin();
    let value = vm.ctx.new_float(value);
    Ok(value)
}

fn math_tan(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let value = objfloat::get_value(value);
    let value = value.tan();
    let value = vm.ctx.new_float(value);
    Ok(value)
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"math".to_string(), ctx.new_scope(None));
    py_mod.set_item("acos", ctx.new_rustfunc(math_acos));
    py_mod.set_item("asin", ctx.new_rustfunc(math_asin));
    py_mod.set_item("atan", ctx.new_rustfunc(math_atan));
    py_mod.set_item("cos", ctx.new_rustfunc(math_cos));
    py_mod.set_item("sin", ctx.new_rustfunc(math_sin));
    py_mod.set_item("tan", ctx.new_rustfunc(math_tan));

    py_mod
}
