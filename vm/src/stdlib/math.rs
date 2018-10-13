/* Math builtin module
 *
 *
 */

use super::super::obj::{objfloat, objtype};
use super::super::pyobject::{
    DictProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::VirtualMachine;

// Helper macro:
macro_rules! make_math_func {
    ( $fname:ident, $fun:ident ) => {
        fn $fname(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
            arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
            let value = objfloat::get_value(value);
            let value = value.$fun();
            let value = vm.ctx.new_float(value);
            Ok(value)
        }
    };
}

// Trigonometric functions:
make_math_func!(math_acos, acos);
make_math_func!(math_asin, asin);
make_math_func!(math_atan, atan);
make_math_func!(math_cos, cos);
make_math_func!(math_sin, sin);
make_math_func!(math_tan, tan);

// Hyperbolic functions:
make_math_func!(math_acosh, acosh);
make_math_func!(math_asinh, asinh);
make_math_func!(math_atanh, atanh);
make_math_func!(math_cosh, cosh);
make_math_func!(math_sinh, sinh);
make_math_func!(math_tanh, tanh);

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"math".to_string(), ctx.new_scope(None));
    py_mod.set_item("acos", ctx.new_rustfunc(math_acos));
    py_mod.set_item("asin", ctx.new_rustfunc(math_asin));
    py_mod.set_item("atan", ctx.new_rustfunc(math_atan));
    py_mod.set_item("cos", ctx.new_rustfunc(math_cos));
    py_mod.set_item("sin", ctx.new_rustfunc(math_sin));
    py_mod.set_item("tan", ctx.new_rustfunc(math_tan));

    py_mod.set_item("acosh", ctx.new_rustfunc(math_acosh));
    py_mod.set_item("asinh", ctx.new_rustfunc(math_asinh));
    py_mod.set_item("atanh", ctx.new_rustfunc(math_atanh));
    py_mod.set_item("cosh", ctx.new_rustfunc(math_cosh));
    py_mod.set_item("sinh", ctx.new_rustfunc(math_sinh));
    py_mod.set_item("tanh", ctx.new_rustfunc(math_tanh));

    py_mod
}
