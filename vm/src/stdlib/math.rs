/* Math builtin module
 *
 *
 */

use statrs::function::erf::{erf, erfc};
use statrs::function::gamma::{gamma, ln_gamma};

use crate::function::PyFuncArgs;
use crate::obj::{objfloat, objtype};
use crate::pyobject::{PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

// Helper macro:
macro_rules! make_math_func {
    ( $fname:ident, $fun:ident ) => {
        fn $fname(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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

fn math_isfinite(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let value = objfloat::make_float(vm, value)?.is_finite();
    Ok(vm.ctx.new_bool(value))
}

fn math_isinf(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let value = objfloat::make_float(vm, value)?.is_infinite();
    Ok(vm.ctx.new_bool(value))
}

fn math_isnan(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let value = objfloat::make_float(vm, value)?.is_nan();
    Ok(vm.ctx.new_bool(value))
}

// Power and logarithmic functions:
make_math_func!(math_exp, exp);
make_math_func!(math_expm1, exp_m1);

fn math_log(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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

fn math_log1p(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, None)]);
    let x = objfloat::make_float(vm, x)?;
    Ok(vm.ctx.new_float((x + 1.0).ln()))
}

make_math_func!(math_log2, log2);
make_math_func!(math_log10, log10);

fn math_pow(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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

fn math_atan2(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(y, None), (x, None)]);
    let y = objfloat::make_float(vm, y)?;
    let x = objfloat::make_float(vm, x)?;
    Ok(vm.ctx.new_float(y.atan2(x)))
}

make_math_func!(math_cos, cos);

fn math_hypot(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, None), (y, None)]);
    let x = objfloat::make_float(vm, x)?;
    let y = objfloat::make_float(vm, y)?;
    Ok(vm.ctx.new_float(x.hypot(y)))
}

make_math_func!(math_sin, sin);
make_math_func!(math_tan, tan);

fn math_degrees(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let x = objfloat::make_float(vm, value)?;
    Ok(vm.ctx.new_float(x * (180.0 / std::f64::consts::PI)))
}

fn math_radians(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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
fn math_erf(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let x = objfloat::make_float(vm, value)?;

    if x.is_nan() {
        Ok(vm.ctx.new_float(x))
    } else {
        Ok(vm.ctx.new_float(erf(x)))
    }
}

fn math_erfc(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    let x = objfloat::make_float(vm, value)?;

    if x.is_nan() {
        Ok(vm.ctx.new_float(x))
    } else {
        Ok(vm.ctx.new_float(erfc(x)))
    }
}

fn math_gamma(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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

fn math_lgamma(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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

fn try_magic_method(func_name: &str, vm: &VirtualMachine, value: &PyObjectRef) -> PyResult {
    if let Ok(method) = vm.get_method(value.clone(), func_name) {
        vm.invoke(method, vec![])
    } else {
        Err(vm.new_type_error(format!(
            "TypeError: type {} doesn't define {} method",
            value.class().name,
            func_name,
        )))
    }
}

fn math_trunc(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    try_magic_method("__trunc__", vm, value)
}

fn math_ceil(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    if objtype::isinstance(value, &vm.ctx.float_type) {
        let v = objfloat::get_value(value);
        Ok(vm.ctx.new_float(v.ceil()))
    } else {
        try_magic_method("__ceil__", vm, value)
    }
}

fn math_floor(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(value, None)]);
    if objtype::isinstance(value, &vm.ctx.float_type) {
        let v = objfloat::get_value(value);
        Ok(vm.ctx.new_float(v.floor()))
    } else {
        try_magic_method("__floor__", vm, value)
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "math", {
        // Number theory functions:
        "fabs" => ctx.new_rustfunc(math_fabs),
        "isfinite" => ctx.new_rustfunc(math_isfinite),
        "isinf" => ctx.new_rustfunc(math_isinf),
        "isnan" => ctx.new_rustfunc(math_isnan),

        // Power and logarithmic functions:
        "exp" => ctx.new_rustfunc(math_exp),
        "expm1" => ctx.new_rustfunc(math_expm1),
        "log" => ctx.new_rustfunc(math_log),
        "log1p" => ctx.new_rustfunc(math_log1p),
        "log2" => ctx.new_rustfunc(math_log2),
        "log10" => ctx.new_rustfunc(math_log10),
        "pow" => ctx.new_rustfunc(math_pow),
        "sqrt" => ctx.new_rustfunc(math_sqrt),

        // Trigonometric functions:
        "acos" => ctx.new_rustfunc(math_acos),
        "asin" => ctx.new_rustfunc(math_asin),
        "atan" => ctx.new_rustfunc(math_atan),
        "atan2" => ctx.new_rustfunc(math_atan2),
        "cos" => ctx.new_rustfunc(math_cos),
        "hypot" => ctx.new_rustfunc(math_hypot),
        "sin" => ctx.new_rustfunc(math_sin),
        "tan" => ctx.new_rustfunc(math_tan),

        "degrees" => ctx.new_rustfunc(math_degrees),
        "radians" => ctx.new_rustfunc(math_radians),

        // Hyperbolic functions:
        "acosh" => ctx.new_rustfunc(math_acosh),
        "asinh" => ctx.new_rustfunc(math_asinh),
        "atanh" => ctx.new_rustfunc(math_atanh),
        "cosh" => ctx.new_rustfunc(math_cosh),
        "sinh" => ctx.new_rustfunc(math_sinh),
        "tanh" => ctx.new_rustfunc(math_tanh),

        // Special functions:
        "erf" => ctx.new_rustfunc(math_erf),
        "erfc" => ctx.new_rustfunc(math_erfc),
        "gamma" => ctx.new_rustfunc(math_gamma),
        "lgamma" => ctx.new_rustfunc(math_lgamma),

        // Rounding functions:
        "trunc" => ctx.new_rustfunc(math_trunc),
        "ceil" => ctx.new_rustfunc(math_ceil),
        "floor" => ctx.new_rustfunc(math_floor),

        // Constants:
        "pi" => ctx.new_float(std::f64::consts::PI), // 3.14159...
        "e" => ctx.new_float(std::f64::consts::E), // 2.71..
        "tau" => ctx.new_float(2.0 * std::f64::consts::PI),
        "inf" => ctx.new_float(std::f64::INFINITY),
        "nan" => ctx.new_float(std::f64::NAN)
    })
}
