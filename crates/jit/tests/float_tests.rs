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
fn test_add() {
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
fn test_add_with_integer() {
    let add = jit_function! { add(a:f64, b:i64) -> f64 => r##"
        def add(a: float, b: int):
            return a + b
    "## };

    assert_approx_eq!(add(5.5, 10), Ok(15.5));
    assert_approx_eq!(add(-4.6, 7), Ok(2.4));
    assert_approx_eq!(add(-5.2, -3), Ok(-8.2));
}

#[test]
fn test_sub() {
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
fn test_sub_with_integer() {
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
fn test_mul() {
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
fn test_mul_with_integer() {
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
fn test_power() {
    let pow = jit_function! { pow(a:f64, b:f64) -> f64 => r##"
        def pow(a:float, b: float):
            return a**b
    "##};
    // Test base cases
    assert_approx_eq!(pow(0.0, 0.0), Ok(1.0));
    assert_approx_eq!(pow(0.0, 1.0), Ok(0.0));
    assert_approx_eq!(pow(1.0, 0.0), Ok(1.0));
    assert_approx_eq!(pow(1.0, 1.0), Ok(1.0));
    assert_approx_eq!(pow(1.0, -1.0), Ok(1.0));
    assert_approx_eq!(pow(-1.0, 0.0), Ok(1.0));
    assert_approx_eq!(pow(-1.0, 1.0), Ok(-1.0));
    assert_approx_eq!(pow(-1.0, -1.0), Ok(-1.0));

    // NaN and Infinity cases
    assert_approx_eq!(pow(f64::NAN, 0.0), Ok(1.0));
    //assert_approx_eq!(pow(f64::NAN, 1.0), Ok(f64::NAN)); // Return the correct answer but fails compare
    //assert_approx_eq!(pow(0.0, f64::NAN), Ok(f64::NAN)); // Return the correct answer but fails compare
    assert_approx_eq!(pow(f64::INFINITY, 0.0), Ok(1.0));
    assert_approx_eq!(pow(f64::INFINITY, 1.0), Ok(f64::INFINITY));
    assert_approx_eq!(pow(f64::INFINITY, f64::INFINITY), Ok(f64::INFINITY));
    // Negative infinity cases:
    // For any exponent of 0.0, the result is 1.0.
    assert_approx_eq!(pow(f64::NEG_INFINITY, 0.0), Ok(1.0));
    // For negative infinity base, when b is an odd integer, result is -infinity;
    // when b is even, result is +infinity.
    assert_approx_eq!(pow(f64::NEG_INFINITY, 1.0), Ok(f64::NEG_INFINITY));
    assert_approx_eq!(pow(f64::NEG_INFINITY, 2.0), Ok(f64::INFINITY));
    assert_approx_eq!(pow(f64::NEG_INFINITY, 3.0), Ok(f64::NEG_INFINITY));
    // Exponent -infinity gives 0.0.
    assert_approx_eq!(pow(f64::NEG_INFINITY, f64::NEG_INFINITY), Ok(0.0));

    // Test positive float base, positive float exponent
    assert_approx_eq!(pow(2.0, 2.0), Ok(4.0));
    assert_approx_eq!(pow(3.0, 3.0), Ok(27.0));
    assert_approx_eq!(pow(4.0, 4.0), Ok(256.0));
    assert_approx_eq!(pow(2.0, 3.0), Ok(8.0));
    assert_approx_eq!(pow(2.0, 4.0), Ok(16.0));
    // Test negative float base, positive float exponent (integral exponents only)
    assert_approx_eq!(pow(-2.0, 2.0), Ok(4.0));
    assert_approx_eq!(pow(-3.0, 3.0), Ok(-27.0));
    assert_approx_eq!(pow(-4.0, 4.0), Ok(256.0));
    assert_approx_eq!(pow(-2.0, 3.0), Ok(-8.0));
    assert_approx_eq!(pow(-2.0, 4.0), Ok(16.0));
    // Test positive float base, positive float exponent
    assert_approx_eq!(pow(2.5, 2.0), Ok(6.25));
    assert_approx_eq!(pow(3.5, 3.0), Ok(42.875));
    assert_approx_eq!(pow(4.5, 4.0), Ok(410.0625));
    assert_approx_eq!(pow(2.5, 3.0), Ok(15.625));
    assert_approx_eq!(pow(2.5, 4.0), Ok(39.0625));
    // Test negative float base, positive float exponent (integral exponents only)
    assert_approx_eq!(pow(-2.5, 2.0), Ok(6.25));
    assert_approx_eq!(pow(-3.5, 3.0), Ok(-42.875));
    assert_approx_eq!(pow(-4.5, 4.0), Ok(410.0625));
    assert_approx_eq!(pow(-2.5, 3.0), Ok(-15.625));
    assert_approx_eq!(pow(-2.5, 4.0), Ok(39.0625));
    // Test positive float base, positive float exponent with non-integral exponents
    assert_approx_eq!(pow(2.0, 2.5), Ok(5.656854249492381));
    assert_approx_eq!(pow(3.0, 3.5), Ok(46.76537180435969));
    assert_approx_eq!(pow(4.0, 4.5), Ok(512.0));
    assert_approx_eq!(pow(2.0, 3.5), Ok(11.313708498984761));
    assert_approx_eq!(pow(2.0, 4.5), Ok(22.627416997969522));
    // Test positive float base, negative float exponent
    assert_approx_eq!(pow(2.0, -2.5), Ok(0.1767766952966369));
    assert_approx_eq!(pow(3.0, -3.5), Ok(0.021383343303319473));
    assert_approx_eq!(pow(4.0, -4.5), Ok(0.001953125));
    assert_approx_eq!(pow(2.0, -3.5), Ok(0.08838834764831845));
    assert_approx_eq!(pow(2.0, -4.5), Ok(0.04419417382415922));
    // Test negative float base, negative float exponent (integral exponents only)
    assert_approx_eq!(pow(-2.0, -2.0), Ok(0.25));
    assert_approx_eq!(pow(-3.0, -3.0), Ok(-0.037037037037037035));
    assert_approx_eq!(pow(-4.0, -4.0), Ok(0.00390625));
    assert_approx_eq!(pow(-2.0, -3.0), Ok(-0.125));
    assert_approx_eq!(pow(-2.0, -4.0), Ok(0.0625));

    // Currently negative float base with non-integral exponent is not supported:
    // assert_approx_eq!(pow(-2.0, 2.5), Ok(5.656854249492381));
    // assert_approx_eq!(pow(-3.0, 3.5), Ok(-46.76537180435969));
    // assert_approx_eq!(pow(-4.0, 4.5), Ok(512.0));
    // assert_approx_eq!(pow(-2.0, -2.5), Ok(0.1767766952966369));
    // assert_approx_eq!(pow(-3.0, -3.5), Ok(0.021383343303319473));
    // assert_approx_eq!(pow(-4.0, -4.5), Ok(0.001953125));

    // Extra cases **NOTE** these are not all working:
    //      * If they are commented in then they work
    //      * If they are commented out with a number that is the current return value it throws vs the expected value
    //      * If they are commented out with a "fail to run" that means I couldn't get them to work, could add a case for really big or small values
    // 1e308^2.0
    assert_approx_eq!(pow(1e308, 2.0), Ok(f64::INFINITY));
    // 1e308^(1e-2)
    assert_approx_eq!(pow(1e308, 1e-2), Ok(1202.2644346174131));
    // 1e-308^2.0
    //assert_approx_eq!(pow(1e-308, 2.0), Ok(0.0));  // --8.403311421507407
    // 1e-308^-2.0
    assert_approx_eq!(pow(1e-308, -2.0), Ok(f64::INFINITY));
    // 1e100^(1e50)
    //assert_approx_eq!(pow(1e100, 1e50), Ok(1.0000000000000002e+150)); // fail to run (Crashes as "illegal hardware instruction")
    // 1e50^(1e-100)
    assert_approx_eq!(pow(1e50, 1e-100), Ok(1.0));
    // 1e308^(-1e2)
    //assert_approx_eq!(pow(1e308, -1e2), Ok(0.0)); // 2.961801792837933e25
    // 1e-308^(1e2)
    //assert_approx_eq!(pow(1e-308, 1e2), Ok(f64::INFINITY)); // 1.6692559244043896e46
    // 1e308^(-1e308)
    // assert_approx_eq!(pow(1e308, -1e308), Ok(0.0)); // fail to run (Crashes as "illegal hardware instruction")
    // 1e-308^(1e308)
    // assert_approx_eq!(pow(1e-308, 1e308), Ok(0.0)); // fail to run (Crashes as "illegal hardware instruction")
}

#[test]
fn test_div() {
    let div = jit_function! { div(a:f64, b:f64) -> f64 => r##"
        def div(a: float, b: float):
            return a / b
    "## };

    assert_approx_eq!(div(5.2, 2.0), Ok(2.6));
    assert_approx_eq!(div(3.4, -1.7), Ok(-2.0));
    assert_eq!(div(1.0, 0.0), Ok(f64::INFINITY));
    assert_eq!(div(1.0, -0.0), Ok(f64::NEG_INFINITY));
    assert_eq!(div(-1.0, 0.0), Ok(f64::NEG_INFINITY));
    assert_eq!(div(-1.0, -0.0), Ok(f64::INFINITY));
    assert_bits_eq!(div(-5.2, f64::NAN), Ok(f64::NAN));
    assert_eq!(div(f64::INFINITY, 2.0), Ok(f64::INFINITY));
    assert_bits_eq!(div(-2.0, f64::NEG_INFINITY), Ok(0.0f64));
    assert_bits_eq!(div(1.0, f64::INFINITY), Ok(0.0f64));
    assert_bits_eq!(div(2.0, f64::NEG_INFINITY), Ok(-0.0f64));
    assert_bits_eq!(div(-1.0, f64::INFINITY), Ok(-0.0f64));
}

#[test]
fn test_div_with_integer() {
    let div = jit_function! { div(a:f64, b:i64) -> f64 => r##"
        def div(a: float, b: int):
            return a / b
    "## };

    assert_approx_eq!(div(5.2, 2), Ok(2.6));
    assert_approx_eq!(div(3.4, -1), Ok(-3.4));
    assert_eq!(div(1.0, 0), Ok(f64::INFINITY));
    assert_eq!(div(1.0, -0), Ok(f64::INFINITY));
    assert_eq!(div(-1.0, 0), Ok(f64::NEG_INFINITY));
    assert_eq!(div(-1.0, -0), Ok(f64::NEG_INFINITY));
    assert_eq!(div(f64::INFINITY, 2), Ok(f64::INFINITY));
    assert_eq!(div(f64::NEG_INFINITY, 3), Ok(f64::NEG_INFINITY));
}

#[test]
fn test_if_bool() {
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
fn test_float_eq() {
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
fn test_float_ne() {
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
fn test_float_gt() {
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
fn test_float_gte() {
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
fn test_float_lt() {
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
fn test_float_lte() {
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
