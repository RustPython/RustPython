/* Math builtin module
 *
 *
 */

use num_bigint::BigInt;
use num_traits::{One, Signed, Zero};
use puruspe::{erf, erfc, gamma, ln_gamma};

use crate::builtins::float::{self, IntoPyFloat, PyFloatRef};
use crate::builtins::int::{self, PyInt, PyIntRef};
use crate::function::{Args, OptionalArg};
use crate::pyobject::{BorrowValue, Either, PyIterable, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;
use rustpython_common::float_ops;

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
    #[pyarg(positional)]
    a: IntoPyFloat,
    #[pyarg(positional)]
    b: IntoPyFloat,
    #[pyarg(named, optional)]
    rel_tol: OptionalArg<IntoPyFloat>,
    #[pyarg(named, optional)]
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

fn math_sqrt(value: IntoPyFloat, vm: &VirtualMachine) -> PyResult<f64> {
    let value = value.to_f64();
    if value.is_sign_negative() {
        return Err(vm.new_value_error("math domain error".to_owned()));
    }
    Ok(value.sqrt())
}

fn math_isqrt(x: PyObjectRef, vm: &VirtualMachine) -> PyResult<BigInt> {
    let index = vm.to_index(&x)?;
    let value = index.borrow_value();

    if value.is_negative() {
        return Err(vm.new_value_error("isqrt() argument must be nonnegative".to_owned()));
    }
    Ok(value.sqrt())
}

// Trigonometric functions:
fn math_acos(x: IntoPyFloat, vm: &VirtualMachine) -> PyResult<f64> {
    let x = x.to_f64();
    if x.is_nan() || (-1.0_f64..=1.0_f64).contains(&x) {
        Ok(x.acos())
    } else {
        Err(vm.new_value_error("math domain error".to_owned()))
    }
}

fn math_asin(x: IntoPyFloat, vm: &VirtualMachine) -> PyResult<f64> {
    let x = x.to_f64();
    if x.is_nan() || (-1.0_f64..=1.0_f64).contains(&x) {
        Ok(x.asin())
    } else {
        Err(vm.new_value_error("math domain error".to_owned()))
    }
}

make_math_func!(math_atan, atan);

fn math_atan2(y: IntoPyFloat, x: IntoPyFloat) -> f64 {
    y.to_f64().atan2(x.to_f64())
}

make_math_func!(math_cos, cos);

fn math_hypot(coordinates: Args<IntoPyFloat>) -> f64 {
    let mut coordinates = IntoPyFloat::vec_into_f64(coordinates.into_vec());
    let mut max = 0.0;
    let mut has_nan = false;
    for f in &mut coordinates {
        *f = f.abs();
        if f.is_nan() {
            has_nan = true;
        } else if *f > max {
            max = *f
        }
    }
    // inf takes precedence over nan
    if max.is_infinite() {
        return max;
    }
    if has_nan {
        return f64::NAN;
    }
    vector_norm(&coordinates, max)
}

fn vector_norm(v: &[f64], max: f64) -> f64 {
    if max == 0.0 || v.len() <= 1 {
        return max;
    }
    let mut csum = 1.0;
    let mut frac = 0.0;
    for &f in v {
        let f = f / max;
        let f = f * f;
        let old = csum;
        csum += f;
        // this seemingly redundant operation is to reduce float rounding errors/inaccuracy
        frac += (old - csum) + f;
    }
    max * f64::sqrt(csum - 1.0 + frac)
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
fn math_acosh(x: IntoPyFloat, vm: &VirtualMachine) -> PyResult<f64> {
    let x = x.to_f64();
    if x.is_sign_negative() || x.is_zero() {
        Err(vm.new_value_error("math domain error".to_owned()))
    } else {
        Ok(x.acosh())
    }
}

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
            value.class().name,
            func_name,
        )
    })?;
    vm.invoke(&method, ())
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
    let result_or_err = try_magic_method("__ceil__", vm, &value);
    if result_or_err.is_err() {
        if let Ok(Some(v)) = float::try_float_opt(&value, &vm) {
            let v = float::try_bigint(v.ceil(), vm)?;
            return Ok(vm.ctx.new_int(v));
        }
    }
    result_or_err
}

/// Applies floor to a float, returning an Integral.
///
/// # Arguments
///
/// * `value` - Either a float or a python object which implements __ceil__
/// * `vm` - Represents the python state.
fn math_floor(value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let result_or_err = try_magic_method("__floor__", vm, &value);
    if result_or_err.is_err() {
        if let Ok(Some(v)) = float::try_float_opt(&value, &vm) {
            let v = float::try_bigint(v.floor(), vm)?;
            return Ok(vm.ctx.new_int(v));
        }
    }
    result_or_err
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
        Either::B(z) => int::to_float(z.borrow_value(), vm)?,
    };
    Ok(value * (2_f64).powf(int::to_float(i.borrow_value(), vm)?))
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

fn math_fsum(iter: PyIterable<IntoPyFloat>, vm: &VirtualMachine) -> PyResult<f64> {
    let mut partials = vec![];
    let mut special_sum = 0.0;
    let mut inf_sum = 0.0;

    for obj in iter.iter(vm)? {
        let mut x = obj?.to_f64();

        let xsave = x;
        let mut j = 0;
        // This inner loop applies `hi`/`lo` summation to each
        // partial so that the list of partial sums remains exact.
        for i in 0..partials.len() {
            let mut y: f64 = partials[i];
            if x.abs() < y.abs() {
                std::mem::swap(&mut x, &mut y);
            }
            // Rounded `x+y` is stored in `hi` with round-off stored in
            // `lo`. Together `hi+lo` are exactly equal to `x+y`.
            let hi = x + y;
            let lo = y - (hi - x);
            if lo != 0.0 {
                partials[j] = lo;
                j += 1;
            }
            x = hi;
        }

        if !x.is_finite() {
            // a nonfinite x could arise either as
            // a result of intermediate overflow, or
            // as a result of a nan or inf in the
            // summands
            if xsave.is_finite() {
                return Err(vm.new_overflow_error("intermediate overflow in fsum".to_owned()));
            }
            if xsave.is_infinite() {
                inf_sum += xsave;
            }
            special_sum += xsave;
            // reset partials
            partials.clear();
        }

        if j >= partials.len() {
            partials.push(x);
        } else {
            partials[j] = x;
            partials.truncate(j + 1);
        }
    }
    if special_sum != 0.0 {
        return if inf_sum.is_nan() {
            Err(vm.new_overflow_error("-inf + inf in fsum".to_owned()))
        } else {
            Ok(special_sum)
        };
    }

    let mut n = partials.len();
    if n > 0 {
        n -= 1;
        let mut hi = partials[n];

        let mut lo = 0.0;
        while n > 0 {
            let x = hi;

            n -= 1;
            let y = partials[n];

            hi = x + y;
            lo = y - (hi - x);
            if lo != 0.0 {
                break;
            }
        }
        if n > 0 && ((lo < 0.0 && partials[n - 1] < 0.0) || (lo > 0.0 && partials[n - 1] > 0.0)) {
            let y = lo + lo;
            let x = hi + y;

            // Make half-even rounding work across multiple partials.
            // Needed so that sum([1e-16, 1, 1e16]) will round-up the last
            // digit to two instead of down to zero (the 1e-16 makes the 1
            // slightly closer to two).  With a potential 1 ULP rounding
            // error fixed-up, math.fsum() can guarantee commutativity.
            #[allow(clippy::float_cmp)]
            if y == x - hi {
                hi = x;
            }
        }

        Ok(hi)
    } else {
        Ok(0.0)
    }
}

fn math_factorial(value: PyIntRef, vm: &VirtualMachine) -> PyResult<BigInt> {
    let value = value.borrow_value();
    let one = BigInt::one();
    if value.is_negative() {
        return Err(vm.new_value_error("factorial() not defined for negative values".to_owned()));
    } else if *value <= one {
        return Ok(one);
    }
    // start from 2, since we know that value > 1 and 1*2=2
    let mut current = one + 1;
    let mut product = BigInt::from(2u8);
    while current < *value {
        current += 1;
        product *= &current;
    }
    Ok(product)
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

fn math_nextafter(x: IntoPyFloat, y: IntoPyFloat) -> f64 {
    float_ops::nextafter(x.to_f64(), y.to_f64())
}

fn math_ulp(x: IntoPyFloat) -> f64 {
    float_ops::ulp(x.to_f64())
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
        "fabs" => named_function!(ctx, math, fabs),
        "isfinite" => named_function!(ctx, math, isfinite),
        "isinf" => named_function!(ctx, math, isinf),
        "isnan" => named_function!(ctx, math, isnan),
        "isclose" => named_function!(ctx, math, isclose),
        "copysign" => named_function!(ctx, math, copysign),

        // Power and logarithmic functions:
        "exp" => named_function!(ctx, math, exp),
        "expm1" => named_function!(ctx, math, expm1),
        "log" => named_function!(ctx, math, log),
        "log1p" => named_function!(ctx, math, log1p),
        "log2" => named_function!(ctx, math, log2),
        "log10" => named_function!(ctx, math, log10),
        "pow" => named_function!(ctx, math, pow),
        "sqrt" => named_function!(ctx, math, sqrt),
        "isqrt" => named_function!(ctx, math, isqrt),

        // Trigonometric functions:
        "acos" => named_function!(ctx, math, acos),
        "asin" => named_function!(ctx, math, asin),
        "atan" => named_function!(ctx, math, atan),
        "atan2" => named_function!(ctx, math, atan2),
        "cos" => named_function!(ctx, math, cos),
        "hypot" => named_function!(ctx, math, hypot),
        "sin" => named_function!(ctx, math, sin),
        "tan" => named_function!(ctx, math, tan),

        "degrees" => named_function!(ctx, math, degrees),
        "radians" => named_function!(ctx, math, radians),

        // Hyperbolic functions:
        "acosh" => named_function!(ctx, math, acosh),
        "asinh" => named_function!(ctx, math, asinh),
        "atanh" => named_function!(ctx, math, atanh),
        "cosh" => named_function!(ctx, math, cosh),
        "sinh" => named_function!(ctx, math, sinh),
        "tanh" => named_function!(ctx, math, tanh),

        // Special functions:
        "erf" => named_function!(ctx, math, erf),
        "erfc" => named_function!(ctx, math, erfc),
        "gamma" => named_function!(ctx, math, gamma),
        "lgamma" => named_function!(ctx, math, lgamma),

        "frexp" => named_function!(ctx, math, frexp),
        "ldexp" => named_function!(ctx, math, ldexp),
        "modf" => named_function!(ctx, math, modf),
        "fmod" => named_function!(ctx, math, fmod),
        "fsum" => named_function!(ctx, math, fsum),
        "remainder" => named_function!(ctx, math, remainder),

        // Rounding functions:
        "trunc" => named_function!(ctx, math, trunc),
        "ceil" => named_function!(ctx, math, ceil),
        "floor" => named_function!(ctx, math, floor),

        // Gcd function
        "gcd" => named_function!(ctx, math, gcd),
        "lcm" => named_function!(ctx, math, lcm),

        // Factorial function
        "factorial" => named_function!(ctx, math, factorial),

        // Floating point
        "nextafter" => named_function!(ctx, math, nextafter),
        "ulp" => named_function!(ctx, math, ulp),

        // Constants:
        "pi" => ctx.new_float(std::f64::consts::PI), // 3.14159...
        "e" => ctx.new_float(std::f64::consts::E), // 2.71..
        "tau" => ctx.new_float(2.0 * std::f64::consts::PI),
        "inf" => ctx.new_float(std::f64::INFINITY),
        "nan" => ctx.new_float(std::f64::NAN)
    })
}
