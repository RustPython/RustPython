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
