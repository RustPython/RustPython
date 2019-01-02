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
    py_item!(ctx, mod math {
        // Number theory functions:
        fn fabs = math_fabs;
        fn isfinite = math_isfinite;
        fn isinf = math_isinf;
        fn isnan = math_isnan;

        // Power and logarithmic functions:
        fn exp = math_exp;
        fn expm1 = math_expm1;
        fn log = math_log;
        fn log1p = math_log1p;
        fn log2 = math_log2;
        fn log10 = math_log10;
        fn pow = math_pow;
        fn sqrt = math_sqrt;

        // Trigonometric functions:
        fn acos = math_acos;
        fn asin = math_asin;
        fn atan = math_atan;
        fn atan2 = math_atan2;
        fn cos = math_cos;
        fn hypot = math_hypot;
        fn sin = math_sin;
        fn tan = math_tan;

        fn degrees = math_degrees;
        fn radians = math_radians;

        // Hyperbolic functions:
        fn acosh = math_acosh;
        fn asinh = math_asinh;
        fn atanh = math_atanh;
        fn cosh = math_cosh;
        fn sinh = math_sinh;
        fn tanh = math_tanh;

        // Special functions:
        fn erf = math_erf;
        fn erfc = math_erfc;
        fn gamma = math_gamma;
        fn lgamma = math_lgamma;
        // Constants:
        let pi = ctx.new_float(std::f64::consts::PI); // 3.14159...
        let e = ctx.new_float(std::f64::consts::E); // 2.71..
        let tau = ctx.new_float(2.0 * std::f64::consts::PI);
        let inf = ctx.new_float(std::f64::INFINITY);
        let nan = ctx.new_float(std::f64::NAN);
    })
}
