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
    assert_eq!(floor_div(-3, 1), Ok(-3));
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
