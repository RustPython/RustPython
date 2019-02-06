/* Math builtin module
 *
 *
 */

use super::super::obj::{objfloat, objtype};
use super::super::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use super::super::VirtualMachine;
use statrs::function::erf::{erf, erfc};
use statrs::function::gamma::{gamma, ln_gamma};
use std;

// Helper macro:
macro_rules! make_math_func {
    ( $fname:ident, $fun:ident ) => {
        fn $fname(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
            arg_check!(vm, args, required = [(value, None)]);
            let value = objfloat::make_float(vm, value)?;
            let value = value.$fun();
            let value = vm.ctx.new_float(value);
            Ok(value)
        }
    };
}

// Number theory functions:
make_math_func!(math_fabs, abs);

fn math_isfinite(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let value = objfloat::make_float(vm, value)?.is_finite();
    Ok(vm.ctx.new_bool(value))
}

fn math_isinf(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let value = objfloat::make_float(vm, value)?.is_infinite();
    Ok(vm.ctx.new_bool(value))
}

fn math_isnan(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let value = objfloat::make_float(vm, value)?.is_nan();
    Ok(vm.ctx.new_bool(value))
}

// Power and logarithmic functions:
make_math_func!(math_exp, exp);
make_math_func!(math_expm1, exp_m1);

fn math_log(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, None)], optional = [(base, None)]);
    let x = objfloat::make_float(vm, x)?;
    match base {
        None => Ok(vm.ctx.new_float(x.ln())),
        Some(base) => {
            let base = objfloat::make_float(vm, base)?;
            Ok(vm.ctx.new_float(x.log(base)))
        }
    }
}

fn math_log1p(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, None)]);
    let x = objfloat::make_float(vm, x)?;
    Ok(vm.ctx.new_float((x + 1.0).ln()))
}

make_math_func!(math_log2, log2);
make_math_func!(math_log10, log10);

fn math_pow(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, None), (y, None)]);
    let x = objfloat::make_float(vm, x)?;
    let y = objfloat::make_float(vm, y)?;
    Ok(vm.ctx.new_float(x.powf(y)))
}

make_math_func!(math_sqrt, sqrt);

// Trigonometric functions:
make_math_func!(math_acos, acos);
make_math_func!(math_asin, asin);
make_math_func!(math_atan, atan);

fn math_atan2(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(y, None), (x, None)]);
    let y = objfloat::make_float(vm, y)?;
    let x = objfloat::make_float(vm, x)?;
    Ok(vm.ctx.new_float(y.atan2(x)))
}

make_math_func!(math_cos, cos);

fn math_hypot(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, None), (y, None)]);
    let x = objfloat::make_float(vm, x)?;
    let y = objfloat::make_float(vm, y)?;
    Ok(vm.ctx.new_float(x.hypot(y)))
}

make_math_func!(math_sin, sin);
make_math_func!(math_tan, tan);

fn math_degrees(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let x = objfloat::make_float(vm, value)?;
    Ok(vm.ctx.new_float(x * (180.0 / std::f64::consts::PI)))
}

fn math_radians(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let x = objfloat::make_float(vm, value)?;
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
    arg_check!(vm, args, required = [(value, None)]);
    let x = objfloat::make_float(vm, value)?;

    if x.is_nan() {
        Ok(vm.ctx.new_float(x))
    } else {
        Ok(vm.ctx.new_float(erf(x)))
    }
}

fn math_erfc(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let x = objfloat::make_float(vm, value)?;

    if x.is_nan() {
        Ok(vm.ctx.new_float(x))
    } else {
        Ok(vm.ctx.new_float(erfc(x)))
    }
}

fn math_gamma(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let x = objfloat::make_float(vm, value)?;

    if x.is_finite() {
        Ok(vm.ctx.new_float(gamma(x)))
    } else if x.is_nan() || x.is_sign_positive() {
        Ok(vm.ctx.new_float(x))
    } else {
        Ok(vm.ctx.new_float(std::f64::NAN))
    }
}

fn math_lgamma(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let x = objfloat::make_float(vm, value)?;

    if x.is_finite() {
        Ok(vm.ctx.new_float(ln_gamma(x)))
    } else if x.is_nan() {
        Ok(vm.ctx.new_float(x))
    } else {
        Ok(vm.ctx.new_float(std::f64::INFINITY))
    }
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"math".to_string(), ctx.new_scope(None));

    // Number theory functions:
    ctx.set_attr(&py_mod, "fabs", ctx.new_rustfunc(math_fabs));
    ctx.set_attr(&py_mod, "isfinite", ctx.new_rustfunc(math_isfinite));
    ctx.set_attr(&py_mod, "isinf", ctx.new_rustfunc(math_isinf));
    ctx.set_attr(&py_mod, "isnan", ctx.new_rustfunc(math_isnan));

    // Power and logarithmic functions:
    ctx.set_attr(&py_mod, "exp", ctx.new_rustfunc(math_exp));
    ctx.set_attr(&py_mod, "expm1", ctx.new_rustfunc(math_expm1));
    ctx.set_attr(&py_mod, "log", ctx.new_rustfunc(math_log));
    ctx.set_attr(&py_mod, "log1p", ctx.new_rustfunc(math_log1p));
    ctx.set_attr(&py_mod, "log2", ctx.new_rustfunc(math_log2));
    ctx.set_attr(&py_mod, "log10", ctx.new_rustfunc(math_log10));
    ctx.set_attr(&py_mod, "pow", ctx.new_rustfunc(math_pow));
    ctx.set_attr(&py_mod, "sqrt", ctx.new_rustfunc(math_sqrt));

    // Trigonometric functions:
    ctx.set_attr(&py_mod, "acos", ctx.new_rustfunc(math_acos));
    ctx.set_attr(&py_mod, "asin", ctx.new_rustfunc(math_asin));
    ctx.set_attr(&py_mod, "atan", ctx.new_rustfunc(math_atan));
    ctx.set_attr(&py_mod, "atan2", ctx.new_rustfunc(math_atan2));
    ctx.set_attr(&py_mod, "cos", ctx.new_rustfunc(math_cos));
    ctx.set_attr(&py_mod, "hypot", ctx.new_rustfunc(math_hypot));
    ctx.set_attr(&py_mod, "sin", ctx.new_rustfunc(math_sin));
    ctx.set_attr(&py_mod, "tan", ctx.new_rustfunc(math_tan));

    ctx.set_attr(&py_mod, "degrees", ctx.new_rustfunc(math_degrees));
    ctx.set_attr(&py_mod, "radians", ctx.new_rustfunc(math_radians));

    // Hyperbolic functions:
    ctx.set_attr(&py_mod, "acosh", ctx.new_rustfunc(math_acosh));
    ctx.set_attr(&py_mod, "asinh", ctx.new_rustfunc(math_asinh));
    ctx.set_attr(&py_mod, "atanh", ctx.new_rustfunc(math_atanh));
    ctx.set_attr(&py_mod, "cosh", ctx.new_rustfunc(math_cosh));
    ctx.set_attr(&py_mod, "sinh", ctx.new_rustfunc(math_sinh));
    ctx.set_attr(&py_mod, "tanh", ctx.new_rustfunc(math_tanh));

    // Special functions:
    ctx.set_attr(&py_mod, "erf", ctx.new_rustfunc(math_erf));
    ctx.set_attr(&py_mod, "erfc", ctx.new_rustfunc(math_erfc));
    ctx.set_attr(&py_mod, "gamma", ctx.new_rustfunc(math_gamma));
    ctx.set_attr(&py_mod, "lgamma", ctx.new_rustfunc(math_lgamma));

    // Constants:
    ctx.set_attr(&py_mod, "pi", ctx.new_float(std::f64::consts::PI)); // 3.14159...
    ctx.set_attr(&py_mod, "e", ctx.new_float(std::f64::consts::E)); // 2.71..
    ctx.set_attr(&py_mod, "tau", ctx.new_float(2.0 * std::f64::consts::PI));
    ctx.set_attr(&py_mod, "inf", ctx.new_float(std::f64::INFINITY));
    ctx.set_attr(&py_mod, "nan", ctx.new_float(std::f64::NAN));

    py_mod
}
