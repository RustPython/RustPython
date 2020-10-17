#[test]
fn test_add() {
    let add = jit_function! { add(a:i64, b:i64) -> i64 => r##"
        def add(a: int, b: int):
            return a + b
    "## };

    assert_eq!(add(5, 10), Ok(15));
    assert_eq!(add(-5, 12), Ok(7));
    assert_eq!(add(-5, -3), Ok(-8));
}

#[test]
fn test_sub() {
    let sub = jit_function! { sub(a:i64, b:i64) -> i64 => r##"
        def sub(a: int, b: int):
            return a - b
    "## };

    assert_eq!(sub(5, 10), Ok(-5));
    assert_eq!(sub(12, 10), Ok(2));
    assert_eq!(sub(7, 10), Ok(-3));
    assert_eq!(sub(-3, -10), Ok(7));
}

#[test]
fn test_mult() {
    let mult = jit_function! { mult(a:i64, b:i64) -> i64 => r##"
        def mult(a: int, b: int):
            return a * b
    "## };

    assert_eq!(mult(1, 2), Ok(2));
    assert_eq!(mult(10, 3), Ok(30));
}

#[test]
fn test_div() {
    let div = jit_function! { div(a:i64, b:i64) -> f64 => r##"
        def div(a: int, b: int):
            return a / b
    "## };

    assert_eq!(div(1, 2), Ok(0.5));
    assert_eq!(div(10, 2), Ok(5.));
}

#[test]
fn test_int_float_mix() {
    let int_float_mix = jit_function! { int_float_mix(a:i64, b:f64) -> f64 => r##"
        def int_float_mix(a: int, b: float):
            return a / b
    "## };

    assert_eq!(int_float_mix(1, 2.), Ok(0.5));
    assert_eq!(int_float_mix(10, 2.), Ok(5.));
}

#[test]
fn test_float_int_mix() {
    let test_float_int_mix = jit_function! { test_float_int_mix(a:f64, b:i64) -> f64 => r##"
        def test_float_int_mix(a: float, b: int):
            return a / b
    "## };

    assert_eq!(test_float_int_mix(1., 2), Ok(0.5));
    assert_eq!(test_float_int_mix(10., 2), Ok(5.));
}
