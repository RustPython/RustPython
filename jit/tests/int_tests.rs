use core::f64;

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
fn test_mul() {
    let mul = jit_function! { mul(a:i64, b:i64) -> i64 => r##"
        def mul(a: int, b: int):
            return a * b
    "## };

    assert_eq!(mul(5, 10), Ok(50));
    assert_eq!(mul(0, 5), Ok(0));
    assert_eq!(mul(5, 0), Ok(0));
    assert_eq!(mul(0, 0), Ok(0));
    assert_eq!(mul(-5, 10), Ok(-50));
    assert_eq!(mul(5, -10), Ok(-50));
    assert_eq!(mul(-5, -10), Ok(50));
    assert_eq!(mul(999999, 999999), Ok(999998000001));
    assert_eq!(mul(i64::MAX, 1), Ok(i64::MAX));
    assert_eq!(mul(1, i64::MAX), Ok(i64::MAX));
}

#[test]

fn test_div() {
    let div = jit_function! { div(a:i64, b:i64) -> f64 => r##"
        def div(a: int, b: int):
            return a / b
    "## };

    assert_eq!(div(0, 1), Ok(0.0));
    assert_eq!(div(5, 1), Ok(5.0));
    assert_eq!(div(5, 10), Ok(0.5));
    assert_eq!(div(5, 2), Ok(2.5));
    assert_eq!(div(12, 10), Ok(1.2));
    assert_eq!(div(7, 10), Ok(0.7));
    assert_eq!(div(-3, -1), Ok(3.0));
    assert_eq!(div(-3, 1), Ok(-3.0));
    assert_eq!(div(1, 1000), Ok(0.001));
    assert_eq!(div(1, 100000), Ok(0.00001));
    assert_eq!(div(2, 3), Ok(0.6666666666666666));
    assert_eq!(div(1, 3), Ok(0.3333333333333333));
    assert_eq!(div(i64::MAX, 2), Ok(4611686018427387904.0));
    assert_eq!(div(i64::MIN, 2), Ok(-4611686018427387904.0));
    assert_eq!(div(i64::MIN, -1), Ok(9223372036854775808.0)); // Overflow case
    assert_eq!(div(i64::MIN, i64::MAX), Ok(-1.0));
}


#[test]
fn test_floor_div() {
    let floor_div = jit_function! { floor_div(a:i64, b:i64) -> i64 => r##"
        def floor_div(a: int, b: int):
            return a // b
    "## };

    assert_eq!(floor_div(5, 10), Ok(0));
    assert_eq!(floor_div(5, 2), Ok(2));
    assert_eq!(floor_div(12, 10), Ok(1));
    assert_eq!(floor_div(7, 10), Ok(0));
    assert_eq!(floor_div(-3, -1), Ok(3));
    assert_eq!(floor_div(-3, 0), Ok(-3));
}

#[test]

fn test_exp() {
    let exp = jit_function! { exp(a: i64, b: i64) -> f64 => r##"
    def exp(a: int, b: int):
        return a ** b
    "## };
    
    assert_eq!(exp(2, 3), Ok(8.0));
    assert_eq!(exp(3, 2), Ok(9.0));
    assert_eq!(exp(5, 0), Ok(1.0));
    assert_eq!(exp(0, 0), Ok(1.0));
    assert_eq!(exp(-5, 0), Ok(1.0));
    assert_eq!(exp(0, 1), Ok(0.0));
    assert_eq!(exp(0, 5), Ok(0.0));
    assert_eq!(exp(-2, 2), Ok(4.0));
    assert_eq!(exp(-3, 4), Ok(81.0));
    assert_eq!(exp(-2, 3), Ok(-8.0));
    assert_eq!(exp(-3, 3), Ok(-27.0));
    assert_eq!(exp(1000, 2), Ok(1000000.0));
}
#[test]
fn test_mod() {
    let modulo = jit_function! { modulo(a:i64, b:i64) -> i64 => r##"
        def modulo(a: int, b: int):
            return a % b
    "## };

    assert_eq!(modulo(5, 10), Ok(5));
    assert_eq!(modulo(5, 2), Ok(1));
    assert_eq!(modulo(12, 10), Ok(2));
    assert_eq!(modulo(7, 10), Ok(7));
    assert_eq!(modulo(-3, 1), Ok(0));
    assert_eq!(modulo(-5, 10), Ok(-5));
}

#[test]
fn test_lshift() {
    let lshift = jit_function! { lshift(a:i64, b:i64) -> i64 => r##"
        def lshift(a: int, b: int):
            return a << b
    "## };

    assert_eq!(lshift(5, 10), Ok(5120));
    assert_eq!(lshift(5, 2), Ok(20));
    assert_eq!(lshift(12, 10), Ok(12288));
    assert_eq!(lshift(7, 10), Ok(7168));
    assert_eq!(lshift(-3, 1), Ok(-6));
    assert_eq!(lshift(-10, 2), Ok(-40));
}

#[test]
fn test_rshift() {
    let rshift = jit_function! { rshift(a:i64, b:i64) -> i64 => r##"
        def rshift(a: int, b: int):
            return a >> b
    "## };

    assert_eq!(rshift(5120, 10), Ok(5));
    assert_eq!(rshift(20, 2), Ok(5));
    assert_eq!(rshift(12288, 10), Ok(12));
    assert_eq!(rshift(7168, 10), Ok(7));
    assert_eq!(rshift(-3, 1), Ok(-2));
    assert_eq!(rshift(-10, 2), Ok(-3));
}

#[test]
fn test_and() {
    let bitand = jit_function! { bitand(a:i64, b:i64) -> i64 => r##"
        def bitand(a: int, b: int):
            return a & b
    "## };

    assert_eq!(bitand(5120, 10), Ok(0));
    assert_eq!(bitand(20, 16), Ok(16));
    assert_eq!(bitand(12488, 4249), Ok(4232));
    assert_eq!(bitand(7168, 2), Ok(0));
    assert_eq!(bitand(-3, 1), Ok(1));
    assert_eq!(bitand(-10, 2), Ok(2));
}

#[test]
fn test_or() {
    let bitor = jit_function! { bitor(a:i64, b:i64) -> i64 => r##"
        def bitor(a: int, b: int):
            return a | b
    "## };

    assert_eq!(bitor(5120, 10), Ok(5130));
    assert_eq!(bitor(20, 16), Ok(20));
    assert_eq!(bitor(12488, 4249), Ok(12505));
    assert_eq!(bitor(7168, 2), Ok(7170));
    assert_eq!(bitor(-3, 1), Ok(-3));
    assert_eq!(bitor(-10, 2), Ok(-10));
}

#[test]
fn test_xor() {
    let bitxor = jit_function! { bitxor(a:i64, b:i64) -> i64 => r##"
        def bitxor(a: int, b: int):
            return a ^ b
    "## };

    assert_eq!(bitxor(5120, 10), Ok(5130));
    assert_eq!(bitxor(20, 16), Ok(4));
    assert_eq!(bitxor(12488, 4249), Ok(8273));
    assert_eq!(bitxor(7168, 2), Ok(7170));
    assert_eq!(bitxor(-3, 1), Ok(-4));
    assert_eq!(bitxor(-10, 2), Ok(-12));
}

#[test]
fn test_eq() {
    let eq = jit_function! { eq(a:i64, b:i64) -> i64 => r##"
        def eq(a: int, b: int):
            if a == b:
                return 1
            return 0
    "## };

    assert_eq!(eq(0, 0), Ok(1));
    assert_eq!(eq(1, 1), Ok(1));
    assert_eq!(eq(0, 1), Ok(0));
    assert_eq!(eq(-200, 200), Ok(0));
}

#[test]
fn test_gt() {
    let gt = jit_function! { gt(a:i64, b:i64) -> i64 => r##"
        def gt(a: int, b: int):
            if a > b:
                return 1
            return 0
    "## };

    assert_eq!(gt(5, 2), Ok(1));
    assert_eq!(gt(2, 5), Ok(0));
    assert_eq!(gt(2, 2), Ok(0));
    assert_eq!(gt(5, 5), Ok(0));
    assert_eq!(gt(-1, -10), Ok(1));
    assert_eq!(gt(1, -1), Ok(1));
}

#[test]
fn test_lt() {
    let lt = jit_function! { lt(a:i64, b:i64) -> i64 => r##"
        def lt(a: int, b: int):
            if a < b:
                return 1
            return 0
    "## };

    assert_eq!(lt(-1, -5), Ok(0));
    assert_eq!(lt(10, 0), Ok(0));
    assert_eq!(lt(0, 1), Ok(1));
    assert_eq!(lt(-10, -1), Ok(1));
    assert_eq!(lt(100, 100), Ok(0));
}

#[test]
fn test_gte() {
    let gte = jit_function! { gte(a:i64, b:i64) -> i64 => r##"
        def gte(a: int, b: int):
            if a >= b:
                return 1
            return 0
    "## };

    assert_eq!(gte(-64, -64), Ok(1));
    assert_eq!(gte(100, -1), Ok(1));
    assert_eq!(gte(1, 2), Ok(0));
    assert_eq!(gte(1, 0), Ok(1));
}

#[test]
fn test_lte() {
    let lte = jit_function! { lte(a:i64, b:i64) -> i64 => r##"
        def lte(a: int, b: int):
            if a <= b:
                return 1
            return 0
    "## };

    assert_eq!(lte(-100, -100), Ok(1));
    assert_eq!(lte(-100, 100), Ok(1));
    assert_eq!(lte(10, 1), Ok(0));
    assert_eq!(lte(0, -2), Ok(0));
}

#[test]
fn test_minus() {
    let minus = jit_function! { minus(a:i64) -> i64 => r##"
        def minus(a: int):
            return -a
    "## };

    assert_eq!(minus(5), Ok(-5));
    assert_eq!(minus(12), Ok(-12));
    assert_eq!(minus(-7), Ok(7));
    assert_eq!(minus(-3), Ok(3));
    assert_eq!(minus(0), Ok(0));
}

#[test]
fn test_plus() {
    let plus = jit_function! { plus(a:i64) -> i64 => r##"
        def plus(a: int):
            return +a
    "## };

    assert_eq!(plus(5), Ok(5));
    assert_eq!(plus(12), Ok(12));
    assert_eq!(plus(-7), Ok(-7));
    assert_eq!(plus(-3), Ok(-3));
    assert_eq!(plus(0), Ok(0));
}

#[test]
fn test_not() {
    let not_ = jit_function! { not_(a: i64) -> bool => r##"
        def not_(a: int):
            return not a
    "## };

    assert_eq!(not_(0), Ok(true));
    assert_eq!(not_(1), Ok(false));
    assert_eq!(not_(-1), Ok(false));
}

