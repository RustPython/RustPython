/* Math builtin module
 *
 *
 */

use statrs::function::erf::{erf, erfc};
use statrs::function::gamma::{gamma, ln_gamma};

use num_bigint::BigInt;
use num_traits::{One, Zero};

use crate::function::{Args, OptionalArg};
use crate::obj::objfloat::{self, IntoPyFloat, PyFloatRef};
use crate::obj::objint::{self, PyInt, PyIntRef};
use crate::obj::objtype;
use crate::pyobject::{BorrowValue, Either, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;
use rustpython_common::float_ops;

#[cfg(not(target_arch = "wasm32"))]
use libc::c_double;

use std::cmp::Ordering;

// Helper macro:
macro_rules! make_math_func {
    ( $fname:ident, $fun:ident ) => {
        fn $fname(value: IntoPyFloat) -> f64 {
            value.to_f64().$fun()
        }
    };
}

macro_rules! make_math_func_bool {
    ( $fname:ident, $fun:ident ) => {
        fn $fname(value: IntoPyFloat) -> bool {
            value.to_f64().$fun()
        }
    };
}

// Number theory functions:
make_math_func!(math_fabs, abs);
make_math_func_bool!(math_isfinite, is_finite);
make_math_func_bool!(math_isinf, is_infinite);
make_math_func_bool!(math_isnan, is_nan);

#[derive(FromArgs)]
struct IsCloseArgs {
    #[pyarg(positional_only, optional = false)]
    a: IntoPyFloat,
    #[pyarg(positional_only, optional = false)]
    b: IntoPyFloat,
    #[pyarg(keyword_only, optional = true)]
    rel_tol: OptionalArg<IntoPyFloat>,
    #[pyarg(keyword_only, optional = true)]
    abs_tol: OptionalArg<IntoPyFloat>,
}

#[allow(clippy::float_cmp)]
fn math_isclose(args: IsCloseArgs, vm: &VirtualMachine) -> PyResult<bool> {
    let a = args.a.to_f64();
    let b = args.b.to_f64();
    let rel_tol = match args.rel_tol {
        OptionalArg::Missing => 1e-09,
        OptionalArg::Present(ref value) => value.to_f64(),
    };

    let abs_tol = match args.abs_tol {
        OptionalArg::Missing => 0.0,
        OptionalArg::Present(ref value) => value.to_f64(),
    };

    if rel_tol < 0.0 || abs_tol < 0.0 {
        return Err(vm.new_value_error("tolerances must be non-negative".to_owned()));
    }

    if a == b {
        /* short circuit exact equality -- needed to catch two infinities of
           the same sign. And perhaps speeds things up a bit sometimes.
        */
        return Ok(true);
    }

    /* This catches the case of two infinities of opposite sign, or
       one infinity and one finite number. Two infinities of opposite
       sign would otherwise have an infinite relative tolerance.
       Two infinities of the same sign are caught by the equality check
       above.
    */

    if a.is_infinite() || b.is_infinite() {
        return Ok(false);
    }

    let diff = (b - a).abs();

    Ok((diff <= (rel_tol * b).abs()) || (diff <= (rel_tol * a).abs()) || (diff <= abs_tol))
}

fn math_copysign(a: IntoPyFloat, b: IntoPyFloat) -> f64 {
    let a = a.to_f64();
    let b = b.to_f64();
    if a.is_nan() || b.is_nan() {
        a
    } else {
        a.copysign(b)
    }
}

// Power and logarithmic functions:
make_math_func!(math_exp, exp);
make_math_func!(math_expm1, exp_m1);

fn math_log(x: IntoPyFloat, base: OptionalArg<IntoPyFloat>) -> f64 {
    base.map_or_else(|| x.to_f64().ln(), |base| x.to_f64().log(base.to_f64()))
}

fn math_log1p(x: IntoPyFloat) -> f64 {
    (x.to_f64() + 1.0).ln()
}

make_math_func!(math_log2, log2);
make_math_func!(math_log10, log10);

fn math_pow(x: IntoPyFloat, y: IntoPyFloat) -> f64 {
    x.to_f64().powf(y.to_f64())
}

make_math_func!(math_sqrt, sqrt);

// Trigonometric functions:
make_math_func!(math_acos, acos);
make_math_func!(math_asin, asin);
make_math_func!(math_atan, atan);

fn math_atan2(y: IntoPyFloat, x: IntoPyFloat) -> f64 {
    y.to_f64().atan2(x.to_f64())
}

make_math_func!(math_cos, cos);

fn math_hypot(x: IntoPyFloat, y: IntoPyFloat) -> f64 {
    x.to_f64().hypot(y.to_f64())
}

make_math_func!(math_sin, sin);
make_math_func!(math_tan, tan);

fn math_degrees(x: IntoPyFloat) -> f64 {
    x.to_f64() * (180.0 / std::f64::consts::PI)
}

fn math_radians(x: IntoPyFloat) -> f64 {
    x.to_f64() * (std::f64::consts::PI / 180.0)
}

// Hyperbolic functions:
make_math_func!(math_acosh, acosh);
make_math_func!(math_asinh, asinh);
make_math_func!(math_atanh, atanh);
make_math_func!(math_cosh, cosh);
make_math_func!(math_sinh, sinh);
make_math_func!(math_tanh, tanh);

// Special functions:
fn math_erf(x: IntoPyFloat) -> f64 {
    let x = x.to_f64();
    if x.is_nan() {
        x
    } else {
        erf(x)
    }
}

fn math_erfc(x: IntoPyFloat) -> f64 {
    let x = x.to_f64();
    if x.is_nan() {
        x
    } else {
        erfc(x)
    }
}

fn math_gamma(x: IntoPyFloat) -> f64 {
    let x = x.to_f64();
    if x.is_finite() {
        gamma(x)
    } else if x.is_nan() || x.is_sign_positive() {
        x
    } else {
        std::f64::NAN
    }
}

fn math_lgamma(x: IntoPyFloat) -> f64 {
    let x = x.to_f64();
    if x.is_finite() {
        ln_gamma(x)
    } else if x.is_nan() {
        x
    } else {
        std::f64::INFINITY
    }
}

fn try_magic_method(func_name: &str, vm: &VirtualMachine, value: &PyObjectRef) -> PyResult {
    let method = vm.get_method_or_type_error(value.clone(), func_name, || {
        format!(
            "type '{}' doesn't define '{}' method",
            value.lease_class().name,
            func_name,
        )
    })?;
    vm.invoke(&method, vec![])
}

fn math_trunc(value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    try_magic_method("__trunc__", vm, &value)
}

/// Applies ceiling to a float, returning an Integral.
///
/// # Arguments
///
/// * `value` - Either a float or a python object which implements __ceil__
/// * `vm` - Represents the python state.
fn math_ceil(value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    if objtype::isinstance(&value, &vm.ctx.types.float_type) {
        let v = objfloat::get_value(&value);
        let v = objfloat::try_bigint(v.ceil(), vm)?;
        Ok(vm.ctx.new_int(v))
    } else {
        try_magic_method("__ceil__", vm, &value)
    }
}

/// Applies floor to a float, returning an Integral.
///
/// # Arguments
///
/// * `value` - Either a float or a python object which implements __ceil__
/// * `vm` - Represents the python state.
fn math_floor(value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    if objtype::isinstance(&value, &vm.ctx.types.float_type) {
        let v = objfloat::get_value(&value);
        let v = objfloat::try_bigint(v.floor(), vm)?;
        Ok(vm.ctx.new_int(v))
    } else {
        try_magic_method("__floor__", vm, &value)
    }
}

fn math_frexp(value: IntoPyFloat) -> (f64, i32) {
    let value = value.to_f64();
    if value.is_finite() {
        let (m, e) = float_ops::ufrexp(value);
        (m * value.signum(), e)
    } else {
        (value, 0)
    }
}

fn math_ldexp(
    value: Either<PyFloatRef, PyIntRef>,
    i: PyIntRef,
    vm: &VirtualMachine,
) -> PyResult<f64> {
    let value = match value {
        Either::A(f) => f.to_f64(),
        Either::B(z) => objint::try_float(z.borrow_value(), vm)?,
    };
    Ok(value * (2_f64).powf(objint::try_float(i.borrow_value(), vm)?))
}

fn math_perf_arb_len_int_op<F>(args: Args<PyIntRef>, op: F, default: BigInt) -> BigInt
where
    F: Fn(&BigInt, &PyInt) -> BigInt,
{
    let argvec = args.into_vec();

    if argvec.is_empty() {
        return default;
    } else if argvec.len() == 1 {
        return op(argvec[0].borrow_value(), &argvec[0]);
    }

    let mut res = argvec[0].borrow_value().clone();
    for num in argvec[1..].iter() {
        res = op(&res, &num)
    }
    res
}

fn math_gcd(args: Args<PyIntRef>) -> BigInt {
    use num_integer::Integer;
    math_perf_arb_len_int_op(args, |x, y| x.gcd(y.borrow_value()), BigInt::zero())
}

fn math_lcm(args: Args<PyIntRef>) -> BigInt {
    use num_integer::Integer;
    math_perf_arb_len_int_op(args, |x, y| x.lcm(y.borrow_value()), BigInt::one())
}

fn math_factorial(value: PyIntRef, vm: &VirtualMachine) -> PyResult<BigInt> {
    let value = value.borrow_value();
    if *value < BigInt::zero() {
        return Err(vm.new_value_error("factorial() not defined for negative values".to_owned()));
    } else if *value <= BigInt::one() {
        return Ok(BigInt::from(1u64));
    }
    let ret: BigInt = num_iter::range_inclusive(BigInt::from(1u64), value.clone()).product();
    Ok(ret)
}

fn math_modf(x: IntoPyFloat) -> (f64, f64) {
    let x = x.to_f64();
    if !x.is_finite() {
        if x.is_infinite() {
            return (0.0_f64.copysign(x), x);
        } else if x.is_nan() {
            return (x, x);
        }
    }

    (x.fract(), x.trunc())
}

#[cfg(not(target_arch = "wasm32"))]
fn math_nextafter(x: IntoPyFloat, y: IntoPyFloat) -> PyResult<f64> {
    extern "C" {
        fn nextafter(x: c_double, y: c_double) -> c_double;
    }
    let x = x.to_f64();
    let y = y.to_f64();
    Ok(unsafe { nextafter(x, y) })
}

#[cfg(target_arch = "wasm32")]
fn math_nextafter(x: IntoPyFloat, y: IntoPyFloat, vm: &VirtualMachine) -> PyResult<f64> {
    Err(vm.new_not_implemented_error("not implemented for this platform".to_owned()))
}

fn fmod(x: f64, y: f64) -> f64 {
    if y.is_infinite() && x.is_finite() {
        return x;
    }

    x % y
}

fn math_fmod(x: IntoPyFloat, y: IntoPyFloat, vm: &VirtualMachine) -> PyResult<f64> {
    let x = x.to_f64();
    let y = y.to_f64();

    let r = fmod(x, y);

    if r.is_nan() && !x.is_nan() && !y.is_nan() {
        return Err(vm.new_value_error("math domain error".to_owned()));
    }

    Ok(r)
}

fn math_remainder(x: IntoPyFloat, y: IntoPyFloat, vm: &VirtualMachine) -> PyResult<f64> {
    let x = x.to_f64();
    let y = y.to_f64();
    if x.is_finite() && y.is_finite() {
        if y == 0.0 {
            return Ok(std::f64::NAN);
        }

        let absx = x.abs();
        let absy = y.abs();
        let modulus = absx % absy;

        let c = absy - modulus;
        let r = match modulus.partial_cmp(&c) {
            Some(Ordering::Less) => modulus,
            Some(Ordering::Greater) => -c,
            _ => modulus - 2.0 * fmod(0.5 * (absx - modulus), absy),
        };

        return Ok(1.0_f64.copysign(x) * r);
    }

    if x.is_nan() {
        return Ok(x);
    }
    if y.is_nan() {
        return Ok(y);
    }
    if x.is_infinite() {
        return Ok(std::f64::NAN);
    }
    if y.is_infinite() {
        return Err(vm.new_value_error("math domain error".to_owned()));
    }
    Ok(x)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "math", {
        // Number theory functions:
        "fabs" => ctx.new_function(math_fabs),
        "isfinite" => ctx.new_function(math_isfinite),
        "isinf" => ctx.new_function(math_isinf),
        "isnan" => ctx.new_function(math_isnan),
        "isclose" => ctx.new_function(math_isclose),
        "copysign" => ctx.new_function(math_copysign),

        // Power and logarithmic functions:
        "exp" => ctx.new_function(math_exp),
        "expm1" => ctx.new_function(math_expm1),
        "log" => ctx.new_function(math_log),
        "log1p" => ctx.new_function(math_log1p),
        "log2" => ctx.new_function(math_log2),
        "log10" => ctx.new_function(math_log10),
        "pow" => ctx.new_function(math_pow),
        "sqrt" => ctx.new_function(math_sqrt),

        // Trigonometric functions:
        "acos" => ctx.new_function(math_acos),
        "asin" => ctx.new_function(math_asin),
        "atan" => ctx.new_function(math_atan),
        "atan2" => ctx.new_function(math_atan2),
        "cos" => ctx.new_function(math_cos),
        "hypot" => ctx.new_function(math_hypot),
        "sin" => ctx.new_function(math_sin),
        "tan" => ctx.new_function(math_tan),

        "degrees" => ctx.new_function(math_degrees),
        "radians" => ctx.new_function(math_radians),

        // Hyperbolic functions:
        "acosh" => ctx.new_function(math_acosh),
        "asinh" => ctx.new_function(math_asinh),
        "atanh" => ctx.new_function(math_atanh),
        "cosh" => ctx.new_function(math_cosh),
        "sinh" => ctx.new_function(math_sinh),
        "tanh" => ctx.new_function(math_tanh),

        // Special functions:
        "erf" => ctx.new_function(math_erf),
        "erfc" => ctx.new_function(math_erfc),
        "gamma" => ctx.new_function(math_gamma),
        "lgamma" => ctx.new_function(math_lgamma),

        "frexp" => ctx.new_function(math_frexp),
        "ldexp" => ctx.new_function(math_ldexp),
        "modf" => ctx.new_function(math_modf),
        "fmod" => ctx.new_function(math_fmod),
        "remainder" => ctx.new_function(math_remainder),

        // Rounding functions:
        "trunc" => ctx.new_function(math_trunc),
        "ceil" => ctx.new_function(math_ceil),
        "floor" => ctx.new_function(math_floor),

        // Gcd function
        "gcd" => ctx.new_function(math_gcd),
        "lcm" => ctx.new_function(math_lcm),

        // Factorial function
        "factorial" => ctx.new_function(math_factorial),

        "nextafter" => ctx.new_function(math_nextafter),

        // Constants:
        "pi" => ctx.new_float(std::f64::consts::PI), // 3.14159...
        "e" => ctx.new_float(std::f64::consts::E), // 2.71..
        "tau" => ctx.new_float(2.0 * std::f64::consts::PI),
        "inf" => ctx.new_float(std::f64::INFINITY),
        "nan" => ctx.new_float(std::f64::NAN)
    })
}
