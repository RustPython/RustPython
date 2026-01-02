pub(crate) use math::make_module;

use crate::{builtins::PyBaseExceptionRef, vm::VirtualMachine};

#[pymodule]
mod math {
    use crate::vm::{
        PyObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
        builtins::{PyFloat, PyInt, PyIntRef, PyStrInterned, try_bigint_to_f64, try_f64_to_bigint},
        function::{ArgIndex, ArgIntoFloat, ArgIterable, Either, OptionalArg, PosArgs},
        identifier,
    };
    use core::cmp::Ordering;
    use itertools::Itertools;
    use malachite_bigint::BigInt;
    use num_traits::{One, Signed, ToPrimitive, Zero};
    use rustpython_common::{float_ops, int::true_div};

    // Constants
    #[pyattr]
    use core::f64::consts::{E as e, PI as pi, TAU as tau};

    use super::pymath_error_to_exception;
    #[pyattr(name = "inf")]
    const INF: f64 = f64::INFINITY;
    #[pyattr(name = "nan")]
    const NAN: f64 = f64::NAN;

    // Helper macro:
    macro_rules! call_math_func {
        ( $fun:ident, $name:ident, $vm:ident ) => {{
            let value = $name.into_float();
            let result = value.$fun();
            result_or_overflow(value, result, $vm)
        }};
    }

    #[inline]
    fn result_or_overflow(value: f64, result: f64, vm: &VirtualMachine) -> PyResult<f64> {
        if !result.is_finite() && value.is_finite() {
            // CPython doesn't return `inf` when called with finite
            // values, it raises OverflowError instead.
            Err(vm.new_overflow_error("math range error"))
        } else {
            Ok(result)
        }
    }

    // Number theory functions:
    #[pyfunction]
    fn fabs(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(abs, x, vm)
    }

    #[pyfunction]
    fn isfinite(x: ArgIntoFloat) -> bool {
        x.into_float().is_finite()
    }

    #[pyfunction]
    fn isinf(x: ArgIntoFloat) -> bool {
        x.into_float().is_infinite()
    }

    #[pyfunction]
    fn isnan(x: ArgIntoFloat) -> bool {
        x.into_float().is_nan()
    }

    #[derive(FromArgs)]
    struct IsCloseArgs {
        #[pyarg(positional)]
        a: ArgIntoFloat,
        #[pyarg(positional)]
        b: ArgIntoFloat,
        #[pyarg(named, optional)]
        rel_tol: OptionalArg<ArgIntoFloat>,
        #[pyarg(named, optional)]
        abs_tol: OptionalArg<ArgIntoFloat>,
    }

    #[pyfunction]
    fn isclose(args: IsCloseArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let a = args.a.into_float();
        let b = args.b.into_float();
        let rel_tol = args.rel_tol.map_or(1e-09, |v| v.into_float());
        let abs_tol = args.abs_tol.map_or(0.0, |v| v.into_float());

        if rel_tol < 0.0 || abs_tol < 0.0 {
            return Err(vm.new_value_error("tolerances must be non-negative"));
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

    #[pyfunction]
    fn copysign(x: ArgIntoFloat, y: ArgIntoFloat) -> f64 {
        x.into_float().copysign(y.into_float())
    }

    // Power and logarithmic functions:
    #[pyfunction]
    fn exp(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(exp, x, vm)
    }

    #[pyfunction]
    fn exp2(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(exp2, x, vm)
    }

    #[pyfunction]
    fn expm1(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(exp_m1, x, vm)
    }

    #[pyfunction]
    fn log(x: PyObjectRef, base: OptionalArg<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<f64> {
        let base: f64 = base.map(Into::into).unwrap_or(core::f64::consts::E);
        if base.is_sign_negative() {
            return Err(vm.new_value_error("math domain error"));
        }
        log2(x, vm).map(|log_x| log_x / base.log2())
    }

    #[pyfunction]
    fn log1p(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        if x.is_nan() || x > -1.0_f64 {
            Ok(x.ln_1p())
        } else {
            Err(vm.new_value_error("math domain error"))
        }
    }

    /// Generates the base-2 logarithm of a BigInt `x`
    fn int_log2(x: &BigInt) -> f64 {
        // log2(x) = log2(2^n * 2^-n * x) = n + log2(x/2^n)
        // If we set 2^n to be the greatest power of 2 below x, then x/2^n is in [1, 2), and can
        // thus be converted into a float.
        let n = x.bits() as u32 - 1;
        let frac = true_div(x, &BigInt::from(2).pow(n));
        f64::from(n) + frac.log2()
    }

    #[pyfunction]
    fn log2(x: PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
        match x.try_float(vm) {
            Ok(x) => {
                let x = x.to_f64();
                if x.is_nan() || x > 0.0_f64 {
                    Ok(x.log2())
                } else {
                    Err(vm.new_value_error("math domain error"))
                }
            }
            Err(float_err) => {
                if let Ok(x) = x.try_int(vm) {
                    let x = x.as_bigint();
                    if x.is_positive() {
                        Ok(int_log2(x))
                    } else {
                        Err(vm.new_value_error("math domain error"))
                    }
                } else {
                    // Return the float error, as it will be more intuitive to users
                    Err(float_err)
                }
            }
        }
    }

    #[pyfunction]
    fn log10(x: PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
        log2(x, vm).map(|log_x| log_x / 10f64.log2())
    }

    #[pyfunction]
    fn pow(x: ArgIntoFloat, y: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        let y = y.into_float();

        if x < 0.0 && x.is_finite() && y.fract() != 0.0 && y.is_finite()
            || x == 0.0 && y < 0.0 && y != f64::NEG_INFINITY
        {
            return Err(vm.new_value_error("math domain error"));
        }

        let value = x.powf(y);

        if x.is_finite() && y.is_finite() && value.is_infinite() {
            return Err(vm.new_overflow_error("math range error"));
        }

        Ok(value)
    }

    #[pyfunction]
    fn sqrt(value: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let value = value.into_float();
        if value.is_nan() {
            return Ok(value);
        }
        if value.is_sign_negative() {
            if value.is_zero() {
                return Ok(-0.0f64);
            }
            return Err(vm.new_value_error("math domain error"));
        }
        Ok(value.sqrt())
    }

    #[pyfunction]
    fn isqrt(x: ArgIndex, vm: &VirtualMachine) -> PyResult<BigInt> {
        let x = x.into_int_ref();
        let value = x.as_bigint();

        if value.is_negative() {
            return Err(vm.new_value_error("isqrt() argument must be nonnegative"));
        }
        Ok(value.sqrt())
    }

    // Trigonometric functions:
    #[pyfunction]
    fn acos(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        if x.is_nan() || (-1.0_f64..=1.0_f64).contains(&x) {
            Ok(x.acos())
        } else {
            Err(vm.new_value_error("math domain error"))
        }
    }

    #[pyfunction]
    fn asin(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        if x.is_nan() || (-1.0_f64..=1.0_f64).contains(&x) {
            Ok(x.asin())
        } else {
            Err(vm.new_value_error("math domain error"))
        }
    }

    #[pyfunction]
    fn atan(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(atan, x, vm)
    }

    #[pyfunction]
    fn atan2(y: ArgIntoFloat, x: ArgIntoFloat) -> f64 {
        y.into_float().atan2(x.into())
    }

    #[pyfunction]
    fn cos(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        if x.is_infinite() {
            return Err(vm.new_value_error("math domain error"));
        }
        result_or_overflow(x, x.cos(), vm)
    }

    #[pyfunction]
    fn hypot(coordinates: PosArgs<ArgIntoFloat>) -> f64 {
        let mut coordinates = ArgIntoFloat::vec_into_f64(coordinates.into_vec());
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
        coordinates.sort_unstable_by(|x, y| x.total_cmp(y).reverse());
        vector_norm(&coordinates)
    }

    /// Implementation of accurate hypotenuse algorithm from Borges 2019.
    /// See https://arxiv.org/abs/1904.09481.
    /// This assumes that its arguments are positive finite and have been scaled to avoid overflow
    /// and underflow.
    fn accurate_hypot(max: f64, min: f64) -> f64 {
        if min <= max * (f64::EPSILON / 2.0).sqrt() {
            return max;
        }
        let hypot = max.mul_add(max, min * min).sqrt();
        let hypot_sq = hypot * hypot;
        let max_sq = max * max;
        let correction = (-min).mul_add(min, hypot_sq - max_sq) + hypot.mul_add(hypot, -hypot_sq)
            - max.mul_add(max, -max_sq);
        hypot - correction / (2.0 * hypot)
    }

    /// Calculates the norm of the vector given by `v`.
    /// `v` is assumed to be a list of non-negative finite floats, sorted in descending order.
    fn vector_norm(v: &[f64]) -> f64 {
        // Drop zeros from the vector.
        let zero_count = v.iter().rev().cloned().take_while(|x| *x == 0.0).count();
        let v = &v[..v.len() - zero_count];
        if v.is_empty() {
            return 0.0;
        }
        if v.len() == 1 {
            return v[0];
        }
        // Calculate scaling to avoid overflow / underflow.
        let max = *v.first().unwrap();
        let min = *v.last().unwrap();
        let scale = if max > (f64::MAX / v.len() as f64).sqrt() {
            max
        } else if min < f64::MIN_POSITIVE.sqrt() {
            // ^ This can be an `else if`, because if the max is near f64::MAX and the min is near
            // f64::MIN_POSITIVE, then the min is relatively unimportant and will be effectively
            // ignored.
            min
        } else {
            1.0
        };
        let mut norm = v
            .iter()
            .copied()
            .map(|x| x / scale)
            .reduce(accurate_hypot)
            .unwrap_or_default();
        if v.len() > 2 {
            // For larger lists of numbers, we can accumulate a rounding error, so a correction is
            // needed, similar to that in `accurate_hypot()`.
            // First, we estimate [sum of squares - norm^2], then we add the first-order
            // approximation of the square root of that to `norm`.
            let correction = v
                .iter()
                .copied()
                .map(|x| (x / scale).powi(2))
                .chain(core::iter::once(-norm * norm))
                // Pairwise summation of floats gives less rounding error than a naive sum.
                .tree_reduce(core::ops::Add::add)
                .expect("expected at least 1 element");
            norm = norm + correction / (2.0 * norm);
        }
        norm * scale
    }

    #[pyfunction]
    fn dist(p: Vec<ArgIntoFloat>, q: Vec<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<f64> {
        let mut max = 0.0;
        let mut has_nan = false;

        let p = ArgIntoFloat::vec_into_f64(p);
        let q = ArgIntoFloat::vec_into_f64(q);
        let mut diffs = vec![];

        if p.len() != q.len() {
            return Err(vm.new_value_error("both points must have the same number of dimensions"));
        }

        for i in 0..p.len() {
            let px = p[i];
            let qx = q[i];

            let x = (px - qx).abs();
            if x.is_nan() {
                has_nan = true;
            }

            diffs.push(x);
            if x > max {
                max = x;
            }
        }

        if max.is_infinite() {
            return Ok(max);
        }
        if has_nan {
            return Ok(f64::NAN);
        }
        diffs.sort_unstable_by(|x, y| x.total_cmp(y).reverse());
        Ok(vector_norm(&diffs))
    }

    #[pyfunction]
    fn sin(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        if x.is_infinite() {
            return Err(vm.new_value_error("math domain error"));
        }
        result_or_overflow(x, x.sin(), vm)
    }

    #[pyfunction]
    fn tan(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        if x.is_infinite() {
            return Err(vm.new_value_error("math domain error"));
        }
        result_or_overflow(x, x.tan(), vm)
    }

    #[pyfunction]
    fn degrees(x: ArgIntoFloat) -> f64 {
        x.into_float() * (180.0 / core::f64::consts::PI)
    }

    #[pyfunction]
    fn radians(x: ArgIntoFloat) -> f64 {
        x.into_float() * (core::f64::consts::PI / 180.0)
    }

    // Hyperbolic functions:

    #[pyfunction]
    fn acosh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        if x.is_sign_negative() || x.is_zero() {
            Err(vm.new_value_error("math domain error"))
        } else {
            Ok(x.acosh())
        }
    }

    #[pyfunction]
    fn asinh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(asinh, x, vm)
    }

    #[pyfunction]
    fn atanh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        if x >= 1.0_f64 || x <= -1.0_f64 {
            Err(vm.new_value_error("math domain error"))
        } else {
            Ok(x.atanh())
        }
    }

    #[pyfunction]
    fn cosh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(cosh, x, vm)
    }

    #[pyfunction]
    fn sinh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(sinh, x, vm)
    }

    #[pyfunction]
    fn tanh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(tanh, x, vm)
    }

    // Special functions:
    #[pyfunction]
    fn erf(x: ArgIntoFloat) -> f64 {
        pymath::erf(x.into())
    }

    #[pyfunction]
    fn erfc(x: ArgIntoFloat) -> f64 {
        pymath::erfc(x.into())
    }

    #[pyfunction]
    fn gamma(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::gamma(x.into()).map_err(|err| pymath_error_to_exception(err, vm))
    }

    #[pyfunction]
    fn lgamma(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::lgamma(x.into()).map_err(|err| pymath_error_to_exception(err, vm))
    }

    fn try_magic_method(
        func_name: &'static PyStrInterned,
        vm: &VirtualMachine,
        value: &PyObject,
    ) -> PyResult {
        let method = vm.get_method_or_type_error(value.to_owned(), func_name, || {
            format!(
                "type '{}' doesn't define '{}' method",
                value.class().name(),
                func_name.as_str(),
            )
        })?;
        method.call((), vm)
    }

    #[pyfunction]
    fn trunc(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_magic_method(identifier!(vm, __trunc__), vm, &x)
    }

    #[pyfunction]
    fn ceil(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let result_or_err = try_magic_method(identifier!(vm, __ceil__), vm, &x);
        if result_or_err.is_err()
            && let Some(v) = x.try_float_opt(vm)
        {
            let v = try_f64_to_bigint(v?.to_f64().ceil(), vm)?;
            return Ok(vm.ctx.new_int(v).into());
        }
        result_or_err
    }

    #[pyfunction]
    fn floor(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let result_or_err = try_magic_method(identifier!(vm, __floor__), vm, &x);
        if result_or_err.is_err()
            && let Some(v) = x.try_float_opt(vm)
        {
            let v = try_f64_to_bigint(v?.to_f64().floor(), vm)?;
            return Ok(vm.ctx.new_int(v).into());
        }
        result_or_err
    }

    #[pyfunction]
    fn frexp(x: ArgIntoFloat) -> (f64, i32) {
        let value: f64 = x.into();
        if value.is_finite() {
            let (m, exp) = float_ops::decompose_float(value);
            (m * value.signum(), exp)
        } else {
            (value, 0)
        }
    }

    #[pyfunction]
    fn ldexp(
        x: Either<PyRef<PyFloat>, PyIntRef>,
        i: PyIntRef,
        vm: &VirtualMachine,
    ) -> PyResult<f64> {
        let value = match x {
            Either::A(f) => f.to_f64(),
            Either::B(z) => try_bigint_to_f64(z.as_bigint(), vm)?,
        };

        if value == 0_f64 || !value.is_finite() {
            // NaNs, zeros and infinities are returned unchanged
            return Ok(value);
        }

        // Using IEEE 754 bit manipulation to handle large exponents correctly.
        // Direct multiplication would overflow for large i values, especially when computing
        // the largest finite float (i=1024, x<1.0). By directly modifying the exponent bits,
        // we avoid intermediate overflow to infinity.

        // Scale subnormals to normal range first, then adjust exponent.
        let (mant, exp0) = if value.abs() < f64::MIN_POSITIVE {
            let scaled = value * (1u64 << 54) as f64; // multiply by 2^54
            let (mant_scaled, exp_scaled) = float_ops::decompose_float(scaled);
            (mant_scaled, exp_scaled - 54) // adjust exponent back
        } else {
            float_ops::decompose_float(value)
        };

        let i_big = i.as_bigint();
        let overflow_bound = BigInt::from(1024_i32 - exp0); // i > 1024 - exp0 => overflow
        if i_big > &overflow_bound {
            return Err(vm.new_overflow_error("math range error"));
        }
        if i_big == &overflow_bound && mant == 1.0 {
            return Err(vm.new_overflow_error("math range error"));
        }
        let underflow_bound = BigInt::from(-1074_i32 - exp0); // i < -1074 - exp0 => 0.0 with sign
        if i_big < &underflow_bound {
            return Ok(0.0f64.copysign(value));
        }

        let i_small: i32 = i_big
            .to_i32()
            .expect("exponent within [-1074-exp0, 1024-exp0] must fit in i32");
        let exp = exp0 + i_small;

        const SIGN_MASK: u64 = 0x8000_0000_0000_0000;
        const FRAC_MASK: u64 = 0x000F_FFFF_FFFF_FFFF;
        let sign_bit: u64 = if value.is_sign_negative() {
            SIGN_MASK
        } else {
            0
        };
        let mant_bits = mant.to_bits() & FRAC_MASK;
        if exp >= -1021 {
            let e_bits = (1022_i32 + exp) as u64;
            let result_bits = sign_bit | (e_bits << 52) | mant_bits;
            return Ok(f64::from_bits(result_bits));
        }

        let full_mant: u64 = (1u64 << 52) | mant_bits;
        let shift: u32 = (-exp - 1021) as u32;
        let frac_shifted = full_mant >> shift;
        let lost_bits = full_mant & ((1u64 << shift) - 1);

        let half = 1u64 << (shift - 1);
        let frac = if (lost_bits > half) || (lost_bits == half && (frac_shifted & 1) == 1) {
            frac_shifted + 1
        } else {
            frac_shifted
        };

        let result_bits = if frac >= (1u64 << 52) {
            sign_bit | (1u64 << 52)
        } else {
            sign_bit | frac
        };
        Ok(f64::from_bits(result_bits))
    }

    fn math_perf_arb_len_int_op<F>(args: PosArgs<ArgIndex>, op: F, default: BigInt) -> BigInt
    where
        F: Fn(&BigInt, &PyInt) -> BigInt,
    {
        let arg_vec = args.into_vec();

        if arg_vec.is_empty() {
            return default;
        } else if arg_vec.len() == 1 {
            return op(arg_vec[0].as_ref().as_bigint(), arg_vec[0].as_ref());
        }

        let mut res = arg_vec[0].as_ref().as_bigint().clone();
        for num in &arg_vec[1..] {
            res = op(&res, num.as_ref())
        }
        res
    }

    #[pyfunction]
    fn gcd(args: PosArgs<ArgIndex>) -> BigInt {
        use num_integer::Integer;
        math_perf_arb_len_int_op(args, |x, y| x.gcd(y.as_bigint()), BigInt::zero())
    }

    #[pyfunction]
    fn lcm(args: PosArgs<ArgIndex>) -> BigInt {
        use num_integer::Integer;
        math_perf_arb_len_int_op(args, |x, y| x.lcm(y.as_bigint()), BigInt::one())
    }

    #[pyfunction]
    fn cbrt(x: ArgIntoFloat) -> f64 {
        x.into_float().cbrt()
    }

    #[pyfunction]
    fn fsum(seq: ArgIterable<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<f64> {
        let mut partials = Vec::with_capacity(32);
        let mut special_sum = 0.0;
        let mut inf_sum = 0.0;

        for obj in seq.iter(vm)? {
            let mut x = obj?.into_float();

            let xsave = x;
            let mut i = 0;
            // This inner loop applies `hi`/`lo` summation to each
            // partial so that the list of partial sums remains exact.
            for j in 0..partials.len() {
                let mut y: f64 = partials[j];
                if x.abs() < y.abs() {
                    core::mem::swap(&mut x, &mut y);
                }
                // Rounded `x+y` is stored in `hi` with round-off stored in
                // `lo`. Together `hi+lo` are exactly equal to `x+y`.
                let hi = x + y;
                let lo = y - (hi - x);
                if lo != 0.0 {
                    partials[i] = lo;
                    i += 1;
                }
                x = hi;
            }

            partials.truncate(i);
            if x != 0.0 {
                if !x.is_finite() {
                    // a non-finite x could arise either as
                    // a result of intermediate overflow, or
                    // as a result of a nan or inf in the
                    // summands
                    if xsave.is_finite() {
                        return Err(vm.new_overflow_error("intermediate overflow in fsum"));
                    }
                    if xsave.is_infinite() {
                        inf_sum += xsave;
                    }
                    special_sum += xsave;
                    // reset partials
                    partials.clear();
                } else {
                    partials.push(x);
                }
            }
        }
        if special_sum != 0.0 {
            return if inf_sum.is_nan() {
                Err(vm.new_value_error("-inf + inf in fsum"))
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
            if n > 0 && ((lo < 0.0 && partials[n - 1] < 0.0) || (lo > 0.0 && partials[n - 1] > 0.0))
            {
                let y = lo + lo;
                let x = hi + y;

                // Make half-even rounding work across multiple partials.
                // Needed so that sum([1e-16, 1, 1e16]) will round-up the last
                // digit to two instead of down to zero (the 1e-16 makes the 1
                // slightly closer to two).  With a potential 1 ULP rounding
                // error fixed-up, math.fsum() can guarantee commutativity.
                if y == x - hi {
                    hi = x;
                }
            }

            Ok(hi)
        } else {
            Ok(0.0)
        }
    }

    #[pyfunction]
    fn factorial(x: PyIntRef, vm: &VirtualMachine) -> PyResult<BigInt> {
        let value = x.as_bigint();
        let one = BigInt::one();
        if value.is_negative() {
            return Err(vm.new_value_error("factorial() not defined for negative values"));
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

    #[pyfunction]
    fn perm(
        n: ArgIndex,
        k: OptionalArg<Option<ArgIndex>>,
        vm: &VirtualMachine,
    ) -> PyResult<BigInt> {
        let n = n.into_int_ref();
        let n = n.as_bigint();
        let k_ref;
        let v = match k.flatten() {
            Some(k) => {
                k_ref = k.into_int_ref();
                k_ref.as_bigint()
            }
            None => n,
        };

        if n.is_negative() || v.is_negative() {
            return Err(vm.new_value_error("perm() not defined for negative values"));
        }
        if v > n {
            return Ok(BigInt::zero());
        }
        let mut result = BigInt::one();
        let mut current = n.clone();
        let tmp = n - v;
        while current > tmp {
            result *= &current;
            current -= 1;
        }
        Ok(result)
    }

    #[pyfunction]
    fn comb(n: ArgIndex, k: ArgIndex, vm: &VirtualMachine) -> PyResult<BigInt> {
        let k = k.into_int_ref();
        let mut k = k.as_bigint();
        let n = n.into_int_ref();
        let n = n.as_bigint();
        let one = BigInt::one();
        let zero = BigInt::zero();

        if n.is_negative() || k.is_negative() {
            return Err(vm.new_value_error("comb() not defined for negative values"));
        }

        let temp = n - k;
        if temp.is_negative() {
            return Ok(zero);
        }

        if temp < *k {
            k = &temp
        }

        if k.is_zero() {
            return Ok(one);
        }

        let mut result = n.clone();
        let mut factor = n.clone();
        let mut current = one;
        while current < *k {
            factor -= 1;
            current += 1;

            result *= &factor;
            result /= &current;
        }

        Ok(result)
    }

    #[pyfunction]
    fn modf(x: ArgIntoFloat) -> (f64, f64) {
        let x = x.into_float();
        if !x.is_finite() {
            if x.is_infinite() {
                return (0.0_f64.copysign(x), x);
            } else if x.is_nan() {
                return (x, x);
            }
        }

        (x.fract(), x.trunc())
    }

    #[derive(FromArgs)]
    struct NextAfterArgs {
        #[pyarg(positional)]
        x: ArgIntoFloat,
        #[pyarg(positional)]
        y: ArgIntoFloat,
        #[pyarg(named, optional)]
        steps: OptionalArg<ArgIndex>,
    }

    #[pyfunction]
    fn nextafter(arg: NextAfterArgs, vm: &VirtualMachine) -> PyResult<f64> {
        let steps: Option<i64> = arg
            .steps
            .map(|v| v.into_int_ref().try_to_primitive(vm))
            .transpose()?
            .into_option();
        let x: f64 = arg.x.into();
        let y: f64 = arg.y.into();
        match steps {
            Some(steps) => {
                if steps < 0 {
                    return Err(vm.new_value_error("steps must be a non-negative integer"));
                }
                Ok(float_ops::nextafter_with_steps(x, y, steps as u64))
            }
            None => Ok(float_ops::nextafter(x, y)),
        }
    }

    #[pyfunction]
    fn ulp(x: ArgIntoFloat) -> f64 {
        float_ops::ulp(x.into())
    }

    fn fmod(x: f64, y: f64) -> f64 {
        if y.is_infinite() && x.is_finite() {
            return x;
        }

        x % y
    }

    #[pyfunction(name = "fmod")]
    fn py_fmod(x: ArgIntoFloat, y: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        let y = y.into_float();

        let r = fmod(x, y);

        if r.is_nan() && !x.is_nan() && !y.is_nan() {
            return Err(vm.new_value_error("math domain error"));
        }

        Ok(r)
    }

    #[pyfunction]
    fn remainder(x: ArgIntoFloat, y: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = x.into_float();
        let y = y.into_float();

        if x.is_finite() && y.is_finite() {
            if y == 0.0 {
                return Err(vm.new_value_error("math domain error"));
            }

            let abs_x = x.abs();
            let abs_y = y.abs();
            let modulus = abs_x % abs_y;

            let c = abs_y - modulus;
            let r = match modulus.partial_cmp(&c) {
                Some(Ordering::Less) => modulus,
                Some(Ordering::Greater) => -c,
                _ => modulus - 2.0 * fmod(0.5 * (abs_x - modulus), abs_y),
            };

            return Ok(1.0_f64.copysign(x) * r);
        }
        if x.is_infinite() && !y.is_nan() {
            return Err(vm.new_value_error("math domain error"));
        }
        if x.is_nan() || y.is_nan() {
            return Ok(f64::NAN);
        }
        if y.is_infinite() {
            Ok(x)
        } else {
            Err(vm.new_value_error("math domain error"))
        }
    }

    #[derive(FromArgs)]
    struct ProdArgs {
        #[pyarg(positional)]
        iterable: ArgIterable<PyObjectRef>,
        #[pyarg(named, optional)]
        start: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn prod(args: ProdArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let iter = args.iterable;

        let mut result = args.start.unwrap_or_else(|| vm.new_pyobj(1));

        // TODO: CPython has optimized implementation for this
        // refer: https://github.com/python/cpython/blob/main/Modules/mathmodule.c#L3093-L3193
        for obj in iter.iter(vm)? {
            let obj = obj?;
            result = vm._mul(&result, &obj)?;
        }

        Ok(result)
    }

    #[pyfunction]
    fn sumprod(
        p: ArgIterable<PyObjectRef>,
        q: ArgIterable<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let mut p_iter = p.iter(vm)?;
        let mut q_iter = q.iter(vm)?;
        // We cannot just create a float because the iterator may contain
        // anything as long as it supports __add__ and __mul__.
        let mut result = vm.new_pyobj(0);
        loop {
            let m_p = p_iter.next();
            let m_q = q_iter.next();
            match (m_p, m_q) {
                (Some(r_p), Some(r_q)) => {
                    let p = r_p?;
                    let q = r_q?;
                    let tmp = vm._mul(&p, &q)?;
                    result = vm._add(&result, &tmp)?;
                }
                (None, None) => break,
                _ => {
                    return Err(vm.new_value_error("Inputs are not the same length"));
                }
            }
        }

        Ok(result)
    }

    #[pyfunction]
    fn fma(
        x: ArgIntoFloat,
        y: ArgIntoFloat,
        z: ArgIntoFloat,
        vm: &VirtualMachine,
    ) -> PyResult<f64> {
        let x = x.into_float();
        let y = y.into_float();
        let z = z.into_float();
        let result = x.mul_add(y, z);

        if result.is_finite() {
            return Ok(result);
        }

        if result.is_nan() {
            if !x.is_nan() && !y.is_nan() && !z.is_nan() {
                return Err(vm.new_value_error("invalid operation in fma"));
            }
        } else if x.is_finite() && y.is_finite() && z.is_finite() {
            return Err(vm.new_overflow_error("overflow in fma"));
        }

        Ok(result)
    }
}

fn pymath_error_to_exception(err: pymath::Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
    match err {
        pymath::Error::EDOM => vm.new_value_error("math domain error"),
        pymath::Error::ERANGE => vm.new_overflow_error("math range error"),
    }
}
