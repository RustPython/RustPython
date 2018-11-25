/* Math builtin module
 *
 *
 */

use super::super::obj::{objfloat, objtype};
use super::super::pyobject::{
    DictProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::VirtualMachine;
use statrs::function::erf::{erf, erfc};
use statrs::function::gamma::{gamma, ln_gamma};
use std;

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

// Number theory functions:
make_math_func!(math_fabs, abs);

fn math_isfinite(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let value = objfloat::get_value(value).is_finite();
    Ok(vm.ctx.new_bool(value))
}

fn math_isinf(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let value = objfloat::get_value(value).is_infinite();
    Ok(vm.ctx.new_bool(value))
}

fn math_isnan(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let value = objfloat::get_value(value).is_nan();
    Ok(vm.ctx.new_bool(value))
}

// Power and logarithmic functions:
make_math_func!(math_exp, exp);
make_math_func!(math_expm1, exp_m1);

fn math_log(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(x, Some(vm.ctx.float_type()))],
        optional = [(base, Some(vm.ctx.float_type()))]
    );
    let x = objfloat::get_value(x);
    match base {
        None => Ok(vm.ctx.new_float(x.ln())),
        Some(base) => {
            let base = objfloat::get_value(base);
            Ok(vm.ctx.new_float(x.log(base)))
        }
    }
}

fn math_log1p(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, Some(vm.ctx.float_type()))]);
    let x = objfloat::get_value(x);
    Ok(vm.ctx.new_float((x + 1.0).ln()))
}

make_math_func!(math_log2, log2);
make_math_func!(math_log10, log10);

fn math_pow(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (x, Some(vm.ctx.float_type())),
            (y, Some(vm.ctx.float_type()))
        ]
    );
    let x = objfloat::get_value(x);
    let y = objfloat::get_value(y);
    Ok(vm.ctx.new_float(x.powf(y)))
}

make_math_func!(math_sqrt, sqrt);

// Trigonometric functions:
make_math_func!(math_acos, acos);
make_math_func!(math_asin, asin);
make_math_func!(math_atan, atan);

fn math_atan2(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (y, Some(vm.ctx.float_type())),
            (x, Some(vm.ctx.float_type()))
        ]
    );
    let y = objfloat::get_value(y);
    let x = objfloat::get_value(x);
    Ok(vm.ctx.new_float(y.atan2(x)))
}

make_math_func!(math_cos, cos);

fn math_hypot(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (x, Some(vm.ctx.float_type())),
            (y, Some(vm.ctx.float_type()))
        ]
    );
    let x = objfloat::get_value(x);
    let y = objfloat::get_value(y);
    Ok(vm.ctx.new_float(x.hypot(y)))
}

make_math_func!(math_sin, sin);
make_math_func!(math_tan, tan);

fn math_degrees(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let x = objfloat::get_value(value);
    Ok(vm.ctx.new_float(x * (180.0 / std::f64::consts::PI)))
}

fn math_radians(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let x = objfloat::get_value(value);
    Ok(vm.ctx.new_float(x * (std::f64::consts::PI / 180.0)))
}

// Hyperbolic functions:
make_math_func!(math_acosh, acosh);
make_math_func!(math_asinh, asinh);
make_math_func!(math_atanh, atanh);
make_math_func!(math_cosh, cosh);
make_math_func!(math_sinh, sinh);
make_math_func!(math_tanh, tanh);

// Special functions:
fn math_erf(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let x = objfloat::get_value(value);

    if x.is_nan() {
        Ok(vm.ctx.new_float(x))
    } else {
        Ok(vm.ctx.new_float(erf(x)))
    }
}

fn math_erfc(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let x = objfloat::get_value(value);

    if x.is_nan() {
        Ok(vm.ctx.new_float(x))
    } else {
        Ok(vm.ctx.new_float(erfc(x)))
    }
}

fn math_gamma(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let x = objfloat::get_value(value);

    if x.is_finite() {
        Ok(vm.ctx.new_float(gamma(x)))
    } else {
        if x.is_nan() || x.is_sign_positive() {
            Ok(vm.ctx.new_float(x))
        } else {
            Ok(vm.ctx.new_float(std::f64::NAN))
        }
    }
}

fn math_lgamma(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, Some(vm.ctx.float_type()))]);
    let x = objfloat::get_value(value);

    if x.is_finite() {
        Ok(vm.ctx.new_float(ln_gamma(x)))
    } else {
        if x.is_nan() {
            Ok(vm.ctx.new_float(x))
        } else {
            Ok(vm.ctx.new_float(std::f64::INFINITY))
        }
    }
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"math".to_string(), ctx.new_scope(None));

    // Number theory functions:
    py_mod.set_item("fabs", ctx.new_rustfunc(math_fabs));
    py_mod.set_item("isfinite", ctx.new_rustfunc(math_isfinite));
    py_mod.set_item("isinf", ctx.new_rustfunc(math_isinf));
    py_mod.set_item("isnan", ctx.new_rustfunc(math_isnan));

    // Power and logarithmic functions:
    py_mod.set_item("exp", ctx.new_rustfunc(math_exp));
    py_mod.set_item("expm1", ctx.new_rustfunc(math_expm1));
    py_mod.set_item("log", ctx.new_rustfunc(math_log));
    py_mod.set_item("log1p", ctx.new_rustfunc(math_log1p));
    py_mod.set_item("log2", ctx.new_rustfunc(math_log2));
    py_mod.set_item("log10", ctx.new_rustfunc(math_log10));
    py_mod.set_item("pow", ctx.new_rustfunc(math_pow));
    py_mod.set_item("sqrt", ctx.new_rustfunc(math_sqrt));

    // Trigonometric functions:
    py_mod.set_item("acos", ctx.new_rustfunc(math_acos));
    py_mod.set_item("asin", ctx.new_rustfunc(math_asin));
    py_mod.set_item("atan", ctx.new_rustfunc(math_atan));
    py_mod.set_item("atan2", ctx.new_rustfunc(math_atan2));
    py_mod.set_item("cos", ctx.new_rustfunc(math_cos));
    py_mod.set_item("hypot", ctx.new_rustfunc(math_hypot));
    py_mod.set_item("sin", ctx.new_rustfunc(math_sin));
    py_mod.set_item("tan", ctx.new_rustfunc(math_tan));

    py_mod.set_item("degrees", ctx.new_rustfunc(math_degrees));
    py_mod.set_item("radians", ctx.new_rustfunc(math_radians));

    // Hyperbolic functions:
    py_mod.set_item("acosh", ctx.new_rustfunc(math_acosh));
    py_mod.set_item("asinh", ctx.new_rustfunc(math_asinh));
    py_mod.set_item("atanh", ctx.new_rustfunc(math_atanh));
    py_mod.set_item("cosh", ctx.new_rustfunc(math_cosh));
    py_mod.set_item("sinh", ctx.new_rustfunc(math_sinh));
    py_mod.set_item("tanh", ctx.new_rustfunc(math_tanh));

    // Special functions:
    py_mod.set_item("erf", ctx.new_rustfunc(math_erf));
    py_mod.set_item("erfc", ctx.new_rustfunc(math_erfc));
    py_mod.set_item("gamma", ctx.new_rustfunc(math_gamma));
    py_mod.set_item("lgamma", ctx.new_rustfunc(math_lgamma));

    // Constants:
    py_mod.set_item("pi", ctx.new_float(std::f64::consts::PI)); // 3.14159...
    py_mod.set_item("e", ctx.new_float(std::f64::consts::E)); // 2.71..
    py_mod.set_item("tau", ctx.new_float(2.0 * std::f64::consts::PI));
    py_mod.set_item("inf", ctx.new_float(std::f64::INFINITY));
    py_mod.set_item("nan", ctx.new_float(std::f64::NAN));

    py_mod
}
