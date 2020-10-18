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
