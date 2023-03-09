pub(crate) use math::make_module;

#[pymodule]
mod math {
    use crate::vm::{
        builtins::{try_bigint_to_f64, try_f64_to_bigint, PyFloat, PyInt, PyIntRef, PyStrInterned},
        function::{ArgIndex, ArgIntoFloat, ArgIterable, Either, OptionalArg, PosArgs},
        identifier, PyObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
    };
    use itertools::Itertools;
    use num_bigint::BigInt;
    use num_rational::Ratio;
    use num_traits::{One, Signed, ToPrimitive, Zero};
    use rustpython_common::float_ops;
    use std::cmp::Ordering;

    // Constants
    #[pyattr]
    use std::f64::consts::{E as e, PI as pi, TAU as tau};
    #[pyattr]
    use std::f64::{INFINITY as inf, NAN as nan};

    // Helper macro:
    macro_rules! call_math_func {
        ( $fun:ident, $name:ident, $vm:ident ) => {{
            let value = *$name;
            let result = value.$fun();
            result_or_overflow(value, result, $vm)
        }};
    }

    #[inline]
    fn result_or_overflow(value: f64, result: f64, vm: &VirtualMachine) -> PyResult<f64> {
        if !result.is_finite() && value.is_finite() {
            // CPython doesn't return `inf` when called with finite
            // values, it raises OverflowError instead.
            Err(vm.new_overflow_error("math range error".to_owned()))
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
        x.is_finite()
    }

    #[pyfunction]
    fn isinf(x: ArgIntoFloat) -> bool {
        x.is_infinite()
    }

    #[pyfunction]
    fn isnan(x: ArgIntoFloat) -> bool {
        x.is_nan()
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
        let a = *args.a;
        let b = *args.b;
        let rel_tol = args.rel_tol.map_or(1e-09, |value| value.into());
        let abs_tol = args.abs_tol.map_or(0.0, |value| value.into());

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

    #[pyfunction]
    fn copysign(x: ArgIntoFloat, y: ArgIntoFloat) -> f64 {
        if x.is_nan() || y.is_nan() {
            x.into()
        } else {
            x.copysign(*y)
        }
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
        let base = base.map(|b| *b).unwrap_or(std::f64::consts::E);
        log2(x, vm).map(|logx| logx / base.log2())
    }

    #[pyfunction]
    fn log1p(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = *x;
        if x.is_nan() || x > -1.0_f64 {
            Ok((x + 1.0_f64).ln())
        } else {
            Err(vm.new_value_error("math domain error".to_owned()))
        }
    }

    /// Generates the base-2 logarithm of a BigInt `x`
    fn int_log2(x: &BigInt) -> f64 {
        // log2(x) = log2(2^n * 2^-n * x) = n + log2(x/2^n)
        // If we set 2^n to be the greatest power of 2 below x, then x/2^n is in [1, 2), and can
        // thus be converted into a float.
        let n = x.bits() as u32 - 1;
        let frac = Ratio::new(x.clone(), BigInt::from(2).pow(n));
        f64::from(n) + frac.to_f64().unwrap().log2()
    }

    #[pyfunction]
    fn log2(x: PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
        match x.try_float(vm) {
            Ok(x) => {
                let x = x.to_f64();
                if x.is_nan() || x > 0.0_f64 {
                    Ok(x.log2())
                } else {
                    Err(vm.new_value_error("math domain error".to_owned()))
                }
            }
            Err(float_err) => {
                if let Ok(x) = x.try_int(vm) {
                    let x = x.as_bigint();
                    if x.is_positive() {
                        Ok(int_log2(x))
                    } else {
                        Err(vm.new_value_error("math domain error".to_owned()))
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
        log2(x, vm).map(|logx| logx / 10f64.log2())
    }

    #[pyfunction]
    fn pow(x: ArgIntoFloat, y: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = *x;
        let y = *y;

        if x < 0.0 && x.is_finite() && y.fract() != 0.0 && y.is_finite() {
            return Err(vm.new_value_error("math domain error".to_owned()));
        }

        if x == 0.0 && y < 0.0 && y != f64::NEG_INFINITY {
            return Err(vm.new_value_error("math domain error".to_owned()));
        }

        let value = x.powf(y);

        Ok(value)
    }

    #[pyfunction]
    fn sqrt(value: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let value = *value;
        if value.is_sign_negative() {
            return Err(vm.new_value_error("math domain error".to_owned()));
        }
        Ok(value.sqrt())
    }

    #[pyfunction]
    fn isqrt(x: ArgIndex, vm: &VirtualMachine) -> PyResult<BigInt> {
        let value = x.as_bigint();

        if value.is_negative() {
            return Err(vm.new_value_error("isqrt() argument must be nonnegative".to_owned()));
        }
        Ok(value.sqrt())
    }

    // Trigonometric functions:
    #[pyfunction]
    fn acos(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = *x;
        if x.is_nan() || (-1.0_f64..=1.0_f64).contains(&x) {
            Ok(x.acos())
        } else {
            Err(vm.new_value_error("math domain error".to_owned()))
        }
    }

    #[pyfunction]
    fn asin(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = *x;
        if x.is_nan() || (-1.0_f64..=1.0_f64).contains(&x) {
            Ok(x.asin())
        } else {
            Err(vm.new_value_error("math domain error".to_owned()))
        }
    }

    #[pyfunction]
    fn atan(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(atan, x, vm)
    }

    #[pyfunction]
    fn atan2(y: ArgIntoFloat, x: ArgIntoFloat) -> f64 {
        y.atan2(*x)
    }

    #[pyfunction]
    fn cos(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(cos, x, vm)
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
                .chain(std::iter::once(-norm * norm))
                // Pairwise summation of floats gives less rounding error than a naive sum.
                .tree_fold1(std::ops::Add::add)
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
            return Err(vm.new_value_error(
                "both points must have the same number of dimensions".to_owned(),
            ));
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
        call_math_func!(sin, x, vm)
    }

    #[pyfunction]
    fn tan(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        call_math_func!(tan, x, vm)
    }

    #[pyfunction]
    fn degrees(x: ArgIntoFloat) -> f64 {
        *x * (180.0 / std::f64::consts::PI)
    }

    #[pyfunction]
    fn radians(x: ArgIntoFloat) -> f64 {
        *x * (std::f64::consts::PI / 180.0)
    }

    // Hyperbolic functions:

    #[pyfunction]
    fn acosh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = *x;
        if x.is_sign_negative() || x.is_zero() {
            Err(vm.new_value_error("math domain error".to_owned()))
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
        let x = *x;
        if x >= 1.0_f64 || x <= -1.0_f64 {
            Err(vm.new_value_error("math domain error".to_owned()))
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
        let x = *x;
        if x.is_nan() {
            x
        } else {
            puruspe::erf(x)
        }
    }

    #[pyfunction]
    fn erfc(x: ArgIntoFloat) -> f64 {
        let x = *x;
        if x.is_nan() {
            x
        } else {
            puruspe::erfc(x)
        }
    }

    #[pyfunction]
    fn gamma(x: ArgIntoFloat) -> f64 {
        let x = *x;
        if x.is_finite() {
            puruspe::gamma(x)
        } else if x.is_nan() || x.is_sign_positive() {
            x
        } else {
            f64::NAN
        }
    }

    #[pyfunction]
    fn lgamma(x: ArgIntoFloat) -> f64 {
        let x = *x;
        if x.is_finite() {
            puruspe::ln_gamma(x)
        } else if x.is_nan() {
            x
        } else {
            f64::INFINITY
        }
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
        if result_or_err.is_err() {
            if let Some(v) = x.try_float_opt(vm) {
                let v = try_f64_to_bigint(v?.to_f64().ceil(), vm)?;
                return Ok(vm.ctx.new_int(v).into());
            }
        }
        result_or_err
    }

    #[pyfunction]
    fn floor(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let result_or_err = try_magic_method(identifier!(vm, __floor__), vm, &x);
        if result_or_err.is_err() {
            if let Some(v) = x.try_float_opt(vm) {
                let v = try_f64_to_bigint(v?.to_f64().floor(), vm)?;
                return Ok(vm.ctx.new_int(v).into());
            }
        }
        result_or_err
    }

    #[pyfunction]
    fn frexp(x: ArgIntoFloat) -> (f64, i32) {
        let value = *x;
        if value.is_finite() {
            let (m, exp) = float_ops::ufrexp(value);
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
            Ok(value)
        } else {
            let result = value * (2_f64).powf(try_bigint_to_f64(i.as_bigint(), vm)?);
            result_or_overflow(value, result, vm)
        }
    }

    fn math_perf_arb_len_int_op<F>(args: PosArgs<ArgIndex>, op: F, default: BigInt) -> BigInt
    where
        F: Fn(&BigInt, &PyInt) -> BigInt,
    {
        let argvec = args.into_vec();

        if argvec.is_empty() {
            return default;
        } else if argvec.len() == 1 {
            return op(argvec[0].as_bigint(), &argvec[0]);
        }

        let mut res = argvec[0].as_bigint().clone();
        for num in &argvec[1..] {
            res = op(&res, num)
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
        x.cbrt()
    }

    #[pyfunction]
    fn fsum(seq: ArgIterable<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<f64> {
        let mut partials = vec![];
        let mut special_sum = 0.0;
        let mut inf_sum = 0.0;

        for obj in seq.iter(vm)? {
            let mut x = *obj?;

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
            return Err(
                vm.new_value_error("factorial() not defined for negative values".to_owned())
            );
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
        let n = n.as_bigint();
        let k_ref;
        let v = match k.flatten() {
            Some(k) => {
                k_ref = k;
                k_ref.as_bigint()
            }
            None => n,
        };

        if n.is_negative() || v.is_negative() {
            return Err(vm.new_value_error("perm() not defined for negative values".to_owned()));
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
        let mut k = k.as_bigint();
        let n = n.as_bigint();
        let one = BigInt::one();
        let zero = BigInt::zero();

        if n.is_negative() || k.is_negative() {
            return Err(vm.new_value_error("comb() not defined for negative values".to_owned()));
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
        let x = *x;
        if !x.is_finite() {
            if x.is_infinite() {
                return (0.0_f64.copysign(x), x);
            } else if x.is_nan() {
                return (x, x);
            }
        }

        (x.fract(), x.trunc())
    }

    #[pyfunction]
    fn nextafter(x: ArgIntoFloat, y: ArgIntoFloat) -> f64 {
        float_ops::nextafter(*x, *y)
    }

    #[pyfunction]
    fn ulp(x: ArgIntoFloat) -> f64 {
        float_ops::ulp(*x)
    }

    fn fmod(x: f64, y: f64) -> f64 {
        if y.is_infinite() && x.is_finite() {
            return x;
        }

        x % y
    }

    #[pyfunction(name = "fmod")]
    fn py_fmod(x: ArgIntoFloat, y: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = *x;
        let y = *y;

        let r = fmod(x, y);

        if r.is_nan() && !x.is_nan() && !y.is_nan() {
            return Err(vm.new_value_error("math domain error".to_owned()));
        }

        Ok(r)
    }

    #[pyfunction]
    fn remainder(x: ArgIntoFloat, y: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let x = *x;
        let y = *y;

        if x.is_finite() && y.is_finite() {
            if y == 0.0 {
                return Err(vm.new_value_error("math domain error".to_owned()));
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
        if x.is_infinite() && !y.is_nan() {
            return Err(vm.new_value_error("math domain error".to_owned()));
        }
        if x.is_nan() || y.is_nan() {
            return Ok(f64::NAN);
        }
        if y.is_infinite() {
            Ok(x)
        } else {
            Err(vm.new_value_error("math domain error".to_owned()))
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

            result = vm
                ._mul(&result, &obj)
                .map_err(|_| vm.new_type_error("math type error".to_owned()))?;
        }

        Ok(result)
    }
}
