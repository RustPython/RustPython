#[cfg(test)]
mod tests {
    macro_rules! assert_approx_eq {
        ($left:expr, $right:expr) => {
            match ($left, $right) {
                (Ok(lhs), Ok(rhs)) => approx::assert_relative_eq!(lhs, rhs),
                (lhs, rhs) => assert_eq!(lhs, rhs),
            }
        };
    }

    macro_rules! assert_bits_eq {
        ($left:expr, $right:expr) => {
            match ($left, $right) {
                (Ok(lhs), Ok(rhs)) => assert!(lhs.to_bits() == rhs.to_bits()),
                (lhs, rhs) => assert_eq!(lhs, rhs),
            }
        };
    }

    #[test]
    fn basic_add() {
        let add = jit_function! { add(a:f64, b:f64) -> f64 => r##"
        def add(a: float, b: float):
            return a + b
    "## };

        assert_approx_eq!(add(5.5, 10.2), Ok(15.7));
        assert_approx_eq!(add(-4.5, 7.6), Ok(3.1));
        assert_approx_eq!(add(-5.2, -3.9), Ok(-9.1));
        assert_bits_eq!(add(-5.2, f64::NAN), Ok(f64::NAN));
        assert_eq!(add(2.0, f64::INFINITY), Ok(f64::INFINITY));
        assert_eq!(add(-2.0, f64::NEG_INFINITY), Ok(f64::NEG_INFINITY));
        assert_eq!(add(1.0, f64::NEG_INFINITY), Ok(f64::NEG_INFINITY));
    }

    #[test]
    fn add_with_integer() {
        let add = jit_function! { add(a:f64, b:i64) -> f64 => r##"
        def add(a: float, b: int):
            return a + b
    "## };

        assert_approx_eq!(add(5.5, 10), Ok(15.5));
        assert_approx_eq!(add(-4.6, 7), Ok(2.4));
        assert_approx_eq!(add(-5.2, -3), Ok(-8.2));
    }

    #[test]
    fn basic_sub() {
        let sub = jit_function! { sub(a:f64, b:f64) -> f64 => r##"
        def sub(a: float, b: float):
            return a - b
    "## };

        assert_approx_eq!(sub(5.2, 3.6), Ok(1.6));
        assert_approx_eq!(sub(3.4, 4.2), Ok(-0.8));
        assert_approx_eq!(sub(-2.1, 1.3), Ok(-3.4));
        assert_approx_eq!(sub(3.1, -1.3), Ok(4.4));
        assert_bits_eq!(sub(-5.2, f64::NAN), Ok(f64::NAN));
        assert_eq!(sub(f64::INFINITY, 2.0), Ok(f64::INFINITY));
        assert_eq!(sub(-2.0, f64::NEG_INFINITY), Ok(f64::INFINITY));
        assert_eq!(sub(1.0, f64::INFINITY), Ok(f64::NEG_INFINITY));
    }

    #[test]
    fn sub_with_integer() {
        let sub = jit_function! { sub(a:i64, b:f64) -> f64 => r##"
        def sub(a: int, b: float):
            return a - b
    "## };

        assert_approx_eq!(sub(5, 3.6), Ok(1.4));
        assert_approx_eq!(sub(3, -4.2), Ok(7.2));
        assert_approx_eq!(sub(-2, 1.3), Ok(-3.3));
        assert_approx_eq!(sub(-3, -1.3), Ok(-1.7));
    }

    #[test]
    fn basic_mul() {
        let mul = jit_function! { mul(a:f64, b:f64) -> f64 => r##"
        def mul(a: float, b: float):
            return a * b
    "## };

        assert_approx_eq!(mul(5.2, 2.0), Ok(10.4));
        assert_approx_eq!(mul(3.4, -1.7), Ok(-5.779999999999999));
        assert_bits_eq!(mul(1.0, 0.0), Ok(0.0f64));
        assert_bits_eq!(mul(1.0, -0.0), Ok(-0.0f64));
        assert_bits_eq!(mul(-1.0, 0.0), Ok(-0.0f64));
        assert_bits_eq!(mul(-1.0, -0.0), Ok(0.0f64));
        assert_bits_eq!(mul(-5.2, f64::NAN), Ok(f64::NAN));
        assert_eq!(mul(1.0, f64::INFINITY), Ok(f64::INFINITY));
        assert_eq!(mul(1.0, f64::NEG_INFINITY), Ok(f64::NEG_INFINITY));
        assert_eq!(mul(-1.0, f64::INFINITY), Ok(f64::NEG_INFINITY));
        assert!(mul(0.0, f64::INFINITY).unwrap().is_nan());
        assert_eq!(mul(f64::NEG_INFINITY, f64::INFINITY), Ok(f64::NEG_INFINITY));
    }

    #[test]
    fn mul_with_integer() {
        let mul = jit_function! { mul(a:f64, b:i64) -> f64 => r##"
        def mul(a: float, b: int):
            return a * b
    "## };

        assert_approx_eq!(mul(5.2, 2), Ok(10.4));
        assert_approx_eq!(mul(3.4, -1), Ok(-3.4));
        assert_bits_eq!(mul(1.0, 0), Ok(0.0f64));
        assert_bits_eq!(mul(-0.0, 1), Ok(-0.0f64));
        assert_bits_eq!(mul(0.0, -1), Ok(-0.0f64));
        assert_bits_eq!(mul(-0.0, -1), Ok(0.0f64));
    }

    #[test]
    fn basic_power() {
        let pow = jit_function! { pow(a:f64, b:f64) -> f64 => r##"
        def pow(a:float, b: float):
            return a**b
    "##};
        // `**` calls `f64::powf` after its guards, the same function the
        // interpreter's `float_pow` calls, so every case below is exact -
        // there is no rounding step of this crate's own that a relative
        // comparison would need to absorb.
        // Test base cases
        assert_bits_eq!(pow(0.0, 0.0), Ok(1.0f64));
        assert_bits_eq!(pow(0.0, 1.0), Ok(0.0f64));
        assert_bits_eq!(pow(1.0, 0.0), Ok(1.0f64));
        assert_bits_eq!(pow(1.0, 1.0), Ok(1.0f64));
        assert_bits_eq!(pow(1.0, -1.0), Ok(1.0f64));
        assert_bits_eq!(pow(-1.0, 0.0), Ok(1.0f64));
        assert_bits_eq!(pow(-1.0, 1.0), Ok(-1.0f64));
        assert_bits_eq!(pow(-1.0, -1.0), Ok(-1.0f64));

        // NaN cases
        assert_bits_eq!(pow(f64::NAN, 0.0), Ok(1.0f64));
        assert_bits_eq!(pow(f64::NAN, 2.0), Ok(f64::NAN));
        assert_bits_eq!(pow(0.0, f64::NAN), Ok(f64::NAN));
        assert_bits_eq!(pow(1.0, f64::NAN), Ok(1.0f64));

        // An infinite exponent does not deoptimize by itself - only an
        // overflowing finite-operand result does, see
        // `float_power_deopts_on_finite_base_overflow` in deopt_tests.rs.
        // `powf` answers these directly, matching the interpreter exactly
        // because both call the same function.
        assert_bits_eq!(pow(f64::INFINITY, f64::INFINITY), Ok(f64::INFINITY));
        assert_bits_eq!(pow(-1.0, f64::INFINITY), Ok(1.0f64));
        assert_bits_eq!(pow(-1.0, f64::NEG_INFINITY), Ok(1.0f64));
        assert_bits_eq!(pow(0.5, f64::INFINITY), Ok(0.0f64));
        assert_bits_eq!(pow(0.5, f64::NEG_INFINITY), Ok(f64::INFINITY));

        // Infinity base cases:
        assert_bits_eq!(pow(f64::INFINITY, 0.0), Ok(1.0f64));
        assert_bits_eq!(pow(f64::INFINITY, 1.0), Ok(f64::INFINITY));
        // An infinite base with a negative exponent correctly returns a
        // signed zero rather than an infinity.
        assert_bits_eq!(pow(f64::INFINITY, -2.0), Ok(0.0f64));
        // Negative infinity cases:
        // For any exponent of 0.0, the result is 1.0.
        assert_bits_eq!(pow(f64::NEG_INFINITY, 0.0), Ok(1.0f64));
        // For negative infinity base, when b is an odd integer, result is -infinity;
        // when b is even, result is +infinity.
        assert_bits_eq!(pow(f64::NEG_INFINITY, 1.0), Ok(f64::NEG_INFINITY));
        assert_bits_eq!(pow(f64::NEG_INFINITY, 2.0), Ok(f64::INFINITY));
        assert_bits_eq!(pow(f64::NEG_INFINITY, 3.0), Ok(f64::NEG_INFINITY));
        // A negative odd exponent keeps the sign but flips the magnitude to
        // a zero, the same as the positive-infinity-base case above.
        assert_bits_eq!(pow(f64::NEG_INFINITY, -3.0), Ok(-0.0f64));
        // An infinite exponent is not special-cased for this base either.
        assert_bits_eq!(pow(f64::NEG_INFINITY, f64::NEG_INFINITY), Ok(0.0f64));

        // A negative zero base keeps its sign rather than being flattened to
        // `+0.0`: `(-0.0) ** 3.0` is `-0.0`.
        assert_bits_eq!(pow(-0.0, 3.0), Ok(-0.0f64));

        // Test positive float base, positive float exponent
        assert_bits_eq!(pow(2.0, 2.0), Ok(4.0f64));
        assert_bits_eq!(pow(3.0, 3.0), Ok(27.0f64));
        assert_bits_eq!(pow(4.0, 4.0), Ok(256.0f64));
        assert_bits_eq!(pow(2.0, 3.0), Ok(8.0f64));
        assert_bits_eq!(pow(2.0, 4.0), Ok(16.0f64));
        // Test negative float base, positive float exponent (integral exponents only)
        assert_bits_eq!(pow(-2.0, 2.0), Ok(4.0f64));
        assert_bits_eq!(pow(-3.0, 3.0), Ok(-27.0f64));
        assert_bits_eq!(pow(-4.0, 4.0), Ok(256.0f64));
        assert_bits_eq!(pow(-2.0, 3.0), Ok(-8.0f64));
        assert_bits_eq!(pow(-2.0, 4.0), Ok(16.0f64));
        // A negative base with an integral exponent is real, so the complex
        // guard must not fire on it.
        assert_bits_eq!(pow(-8.0, 2.0), Ok(64.0f64));
        // Test positive float base, positive float exponent
        assert_bits_eq!(pow(2.5, 2.0), Ok(6.25f64));
        assert_bits_eq!(pow(3.5, 3.0), Ok(42.875f64));
        assert_bits_eq!(pow(4.5, 4.0), Ok(410.0625f64));
        assert_bits_eq!(pow(2.5, 3.0), Ok(15.625f64));
        assert_bits_eq!(pow(2.5, 4.0), Ok(39.0625f64));
        // Test negative float base, positive float exponent (integral exponents only)
        assert_bits_eq!(pow(-2.5, 2.0), Ok(6.25f64));
        assert_bits_eq!(pow(-3.5, 3.0), Ok(-42.875f64));
        assert_bits_eq!(pow(-4.5, 4.0), Ok(410.0625f64));
        assert_bits_eq!(pow(-2.5, 3.0), Ok(-15.625f64));
        assert_bits_eq!(pow(-2.5, 4.0), Ok(39.0625f64));
        // Test positive float base, positive float exponent with non-integral exponents
        assert_bits_eq!(pow(2.0, 2.5), Ok(5.656854249492381f64));
        assert_bits_eq!(pow(3.0, 3.5), Ok(46.76537180435969f64));
        assert_bits_eq!(pow(4.0, 4.5), Ok(512.0f64));
        assert_bits_eq!(pow(2.0, 3.5), Ok(11.313708498984761f64));
        assert_bits_eq!(pow(2.0, 4.5), Ok(22.627416997969522f64));
        // Test positive float base, negative float exponent
        assert_bits_eq!(pow(2.0, -2.5), Ok(0.1767766952966369f64));
        assert_bits_eq!(pow(3.0, -3.5), Ok(0.021383343303319473f64));
        assert_bits_eq!(pow(4.0, -4.5), Ok(0.001953125f64));
        assert_bits_eq!(pow(2.0, -3.5), Ok(0.08838834764831845f64));
        assert_bits_eq!(pow(2.0, -4.5), Ok(0.04419417382415922f64));
        // Test negative float base, negative float exponent (integral exponents only)
        assert_bits_eq!(pow(-2.0, -2.0), Ok(0.25f64));
        assert_bits_eq!(pow(-3.0, -3.0), Ok(-0.037037037037037035f64));
        assert_bits_eq!(pow(-4.0, -4.0), Ok(0.00390625f64));
        assert_bits_eq!(pow(-2.0, -3.0), Ok(-0.125f64));
        assert_bits_eq!(pow(-2.0, -4.0), Ok(0.0625f64));

        // A negative base raised to a non-integral exponent is complex,
        // which this crate cannot produce, so it deoptimizes instead - see
        // `float_power_deopts_on_negative_base_fractional_exponent` in
        // deopt_tests.rs.

        // Extreme magnitudes, finite on both sides:
        assert_bits_eq!(pow(1e308, 1e-2), Ok(1202.2644346174131f64));
        assert_bits_eq!(pow(1e50, 1e-100), Ok(1.0f64));
        // 1e308 ** 2.0 overflows a finite base to an infinity, which raises
        // OverflowError rather than saturating - see
        // `float_power_deopts_on_finite_base_overflow` in deopt_tests.rs.
        // Underflowing all the way to zero, in both directions, does not
        // deoptimize - only an overflow to infinity does.
        assert_bits_eq!(pow(1e-308, 2.0), Ok(0.0f64));
        assert_bits_eq!(pow(1e308, -1e2), Ok(0.0f64));
        assert_bits_eq!(pow(1e-308, 1e2), Ok(0.0f64));
        assert_bits_eq!(pow(1e308, -1e308), Ok(0.0f64));
        assert_bits_eq!(pow(1e-308, 1e308), Ok(0.0f64));
    }

    /// The lowering used to answer float `**` with a hand-rolled
    /// double–double `ln`/`exp`, which lost whole significant digits on a
    /// base far from 1: `1023.0 ** 1.0` came back as `1022.9277018310074`,
    /// and the error stayed small enough at well-scaled inputs that
    /// `assert_approx_eq!`'s relative tolerance in `basic_power` above never
    /// caught it. Calling `f64::powf` directly cannot drift from the
    /// interpreter this way: both sides run the same function on the same
    /// bits. Four of these 24 pairs overflow a finite base and exponent to
    /// an infinity and deoptimize instead of returning; see
    /// `float_power_deopts_on_finite_base_overflow` in deopt_tests.rs.
    #[test]
    fn float_power_matches_far_from_one() {
        let pow = jit_function! { pow(a:f64, b:f64) -> f64 => r##"
        def pow(a:float, b: float):
            return a**b
    "##};
        assert_bits_eq!(pow(1023.0, 1.0), Ok(1023.0f64));
        assert_bits_eq!(pow(1023.0, 2.0), Ok(1046529.0f64));
        assert_bits_eq!(pow(1023.0, -2.0), Ok(9.555396935966418e-7f64));
        assert_bits_eq!(pow(1023.0, 4.0), Ok(1095222947841.0f64));
        assert_bits_eq!(pow(1023.0, -320.0), Ok(0.0f64));
        assert_bits_eq!(pow(1023.0, 0.5), Ok(31.984371183438952f64));
        assert_bits_eq!(pow(1e-308, 1.0), Ok(1e-308f64));
        assert_bits_eq!(pow(1e-308, 2.0), Ok(0.0f64));
        // (1e-308, -2.0) overflows - see deopt_tests.rs.
        assert_bits_eq!(pow(1e-308, 4.0), Ok(0.0f64));
        // (1e-308, -320.0) overflows - see deopt_tests.rs.
        assert_bits_eq!(pow(1e-308, 0.5), Ok(1e-154f64));
        assert_bits_eq!(pow(1e-100, 1.0), Ok(1e-100f64));
        assert_bits_eq!(pow(1e-100, 2.0), Ok(1e-200f64));
        assert_bits_eq!(pow(1e-100, -2.0), Ok(1e200f64));
        assert_bits_eq!(pow(1e-100, 4.0), Ok(0.0f64));
        // (1e-100, -320.0) overflows - see deopt_tests.rs.
        assert_bits_eq!(pow(1e-100, 0.5), Ok(1e-50f64));
        assert_bits_eq!(pow(1e100, 1.0), Ok(1e100f64));
        assert_bits_eq!(pow(1e100, 2.0), Ok(1e200f64));
        assert_bits_eq!(pow(1e100, -2.0), Ok(1e-200f64));
        // (1e100, 4.0) overflows - see deopt_tests.rs.
        assert_bits_eq!(pow(1e100, -320.0), Ok(0.0f64));
        assert_bits_eq!(pow(1e100, 0.5), Ok(1e50f64));
    }

    #[test]
    fn basic_div() {
        let div = jit_function! { div(a:f64, b:f64) -> f64 => r##"
        def div(a: float, b: float):
            return a / b
    "## };

        assert_approx_eq!(div(5.2, 2.0), Ok(2.6));
        assert_approx_eq!(div(4.0, 2.0), Ok(2.0));
        assert_approx_eq!(div(3.4, -1.7), Ok(-2.0));
        // Division by zero raises rather than returning an infinity, so it
        // deoptimizes instead - see deopt_tests.rs.
        assert_bits_eq!(div(-5.2, f64::NAN), Ok(f64::NAN));
        assert_eq!(div(f64::INFINITY, 2.0), Ok(f64::INFINITY));
        assert_bits_eq!(div(-2.0, f64::NEG_INFINITY), Ok(0.0f64));
        assert_bits_eq!(div(1.0, f64::INFINITY), Ok(0.0f64));
        assert_bits_eq!(div(2.0, f64::NEG_INFINITY), Ok(-0.0f64));
        assert_bits_eq!(div(-1.0, f64::INFINITY), Ok(-0.0f64));
    }

    #[test]
    fn div_with_integer() {
        let div = jit_function! { div(a:f64, b:i64) -> f64 => r##"
        def div(a: float, b: int):
            return a / b
    "## };

        assert_approx_eq!(div(5.2, 2), Ok(2.6));
        assert_approx_eq!(div(3.4, -1), Ok(-3.4));
        // Division by zero raises rather than returning an infinity, so it
        // deoptimizes instead - see deopt_tests.rs.
        assert_eq!(div(f64::INFINITY, 2), Ok(f64::INFINITY));
        assert_eq!(div(f64::NEG_INFINITY, 3), Ok(f64::NEG_INFINITY));
    }

    #[test]
    fn basic_if_bool() {
        let if_bool = jit_function! { if_bool(a:f64) -> i64 => r##"
        def if_bool(a: float):
            if a:
                return 1
            return 0
    "## };

        assert_eq!(if_bool(5.2), Ok(1));
        assert_eq!(if_bool(-3.4), Ok(1));
        assert_eq!(if_bool(f64::NAN), Ok(1));
        assert_eq!(if_bool(f64::INFINITY), Ok(1));

        assert_eq!(if_bool(0.0), Ok(0));
    }

    #[test]
    fn basic_float_eq() {
        let float_eq = jit_function! { float_eq(a: f64, b: f64) -> bool => r##"
        def float_eq(a: float, b: float):
            return a == b
    "## };

        assert_eq!(float_eq(2.0, 2.0), Ok(true));
        assert_eq!(float_eq(3.4, -1.7), Ok(false));
        assert_eq!(float_eq(0.0, 0.0), Ok(true));
        assert_eq!(float_eq(-0.0, -0.0), Ok(true));
        assert_eq!(float_eq(-0.0, 0.0), Ok(true));
        assert_eq!(float_eq(-5.2, f64::NAN), Ok(false));
        assert_eq!(float_eq(f64::NAN, f64::NAN), Ok(false));
        assert_eq!(float_eq(f64::INFINITY, f64::NEG_INFINITY), Ok(false));
    }

    #[test]
    fn basic_float_ne() {
        let float_ne = jit_function! { float_ne(a: f64, b: f64) -> bool => r##"
        def float_ne(a: float, b: float):
            return a != b
    "## };

        assert_eq!(float_ne(2.0, 2.0), Ok(false));
        assert_eq!(float_ne(3.4, -1.7), Ok(true));
        assert_eq!(float_ne(0.0, 0.0), Ok(false));
        assert_eq!(float_ne(-0.0, -0.0), Ok(false));
        assert_eq!(float_ne(-0.0, 0.0), Ok(false));
        assert_eq!(float_ne(-5.2, f64::NAN), Ok(true));
        assert_eq!(float_ne(f64::NAN, f64::NAN), Ok(true));
        assert_eq!(float_ne(f64::INFINITY, f64::NEG_INFINITY), Ok(true));
    }

    #[test]
    fn basic_float_gt() {
        let float_gt = jit_function! { float_gt(a: f64, b: f64) -> bool => r##"
        def float_gt(a: float, b: float):
            return a > b
    "## };

        assert_eq!(float_gt(2.0, 2.0), Ok(false));
        assert_eq!(float_gt(3.4, -1.7), Ok(true));
        assert_eq!(float_gt(0.0, 0.0), Ok(false));
        assert_eq!(float_gt(-0.0, -0.0), Ok(false));
        assert_eq!(float_gt(-0.0, 0.0), Ok(false));
        assert_eq!(float_gt(-5.2, f64::NAN), Ok(false));
        assert_eq!(float_gt(f64::NAN, f64::NAN), Ok(false));
        assert_eq!(float_gt(f64::INFINITY, f64::NEG_INFINITY), Ok(true));
    }

    #[test]
    fn basic_float_gte() {
        let float_gte = jit_function! { float_gte(a: f64, b: f64) -> bool => r##"
        def float_gte(a: float, b: float):
            return a >= b
    "## };

        assert_eq!(float_gte(2.0, 2.0), Ok(true));
        assert_eq!(float_gte(3.4, -1.7), Ok(true));
        assert_eq!(float_gte(0.0, 0.0), Ok(true));
        assert_eq!(float_gte(-0.0, -0.0), Ok(true));
        assert_eq!(float_gte(-0.0, 0.0), Ok(true));
        assert_eq!(float_gte(-5.2, f64::NAN), Ok(false));
        assert_eq!(float_gte(f64::NAN, f64::NAN), Ok(false));
        assert_eq!(float_gte(f64::INFINITY, f64::NEG_INFINITY), Ok(true));
    }

    #[test]
    fn basic_float_lt() {
        let float_lt = jit_function! { float_lt(a: f64, b: f64) -> bool => r##"
        def float_lt(a: float, b: float):
            return a < b
    "## };

        assert_eq!(float_lt(2.0, 2.0), Ok(false));
        assert_eq!(float_lt(3.4, -1.7), Ok(false));
        assert_eq!(float_lt(0.0, 0.0), Ok(false));
        assert_eq!(float_lt(-0.0, -0.0), Ok(false));
        assert_eq!(float_lt(-0.0, 0.0), Ok(false));
        assert_eq!(float_lt(-5.2, f64::NAN), Ok(false));
        assert_eq!(float_lt(f64::NAN, f64::NAN), Ok(false));
        assert_eq!(float_lt(f64::INFINITY, f64::NEG_INFINITY), Ok(false));
    }

    #[test]
    fn basic_float_lte() {
        let float_lte = jit_function! { float_lte(a: f64, b: f64) -> bool => r##"
        def float_lte(a: float, b: float):
            return a <= b
    "## };

        assert_eq!(float_lte(2.0, 2.0), Ok(true));
        assert_eq!(float_lte(3.4, -1.7), Ok(false));
        assert_eq!(float_lte(0.0, 0.0), Ok(true));
        assert_eq!(float_lte(-0.0, -0.0), Ok(true));
        assert_eq!(float_lte(-0.0, 0.0), Ok(true));
        assert_eq!(float_lte(-5.2, f64::NAN), Ok(false));
        assert_eq!(float_lte(f64::NAN, f64::NAN), Ok(false));
        assert_eq!(float_lte(f64::INFINITY, f64::NEG_INFINITY), Ok(false));
    }

    #[test]
    fn recursive_float() {
        let recursive_float = jit_function! { recursive_float(n: i64) -> f64 => r##"
        def recursive_float(n: int) -> float:
            if n == 0:
                return 1.0
            return recursive_float(n - 1) / 2.0
    "## };

        assert_eq!(recursive_float(0), Ok(1.0));
        assert_eq!(recursive_float(1), Ok(0.5));
        assert_eq!(recursive_float(4), Ok(0.0625));
    }
}
