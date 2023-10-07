#[test]
fn test_return() {
    let return_ = jit_function! { return_(a: bool) -> bool => r##"
        def return_(a: bool):
            return a
    "## };

    assert_eq!(return_(true), Ok(true));
    assert_eq!(return_(false), Ok(false));
}

#[test]
fn test_const() {
    let const_true = jit_function! { const_true(a: i64) -> bool => r##"
        def const_true(a: int):
            return True
    "## };
    assert_eq!(const_true(0), Ok(true));

    let const_false = jit_function! { const_false(a: i64) -> bool => r##"
        def const_false(a: int):
            return False
    "## };
    assert_eq!(const_false(0), Ok(false));
}

#[test]
fn test_not() {
    let not_ = jit_function! { not_(a: bool) -> bool => r##"
        def not_(a: bool):
            return not a
    "## };

    assert_eq!(not_(true), Ok(false));
    assert_eq!(not_(false), Ok(true));
}

#[test]
fn test_if_not() {
    let if_not = jit_function! { if_not(a: bool) -> i64 => r##"
        def if_not(a: bool):
            if not a:
                return 0
            else:
                return 1

            return -1
    "## };

    assert_eq!(if_not(true), Ok(1));
    assert_eq!(if_not(false), Ok(0));
}

#[test]
fn test_eq() {
    let eq = jit_function! { eq(a:bool, b:bool) -> i64 => r##"
        def eq(a: bool, b: bool):
            if a == b:
                return 1
            return 0
    "## };

    assert_eq!(eq(false, false), Ok(1));
    assert_eq!(eq(true, true), Ok(1));
    assert_eq!(eq(false, true), Ok(0));
    assert_eq!(eq(true, false), Ok(0));
}

#[test]
fn test_eq_with_integers() {
    let eq = jit_function! { eq(a:bool, b:i64) -> i64 => r##"
        def eq(a: bool, b: int):
            if a == b:
                return 1
            return 0
    "## };

    assert_eq!(eq(false, 0), Ok(1));
    assert_eq!(eq(true, 1), Ok(1));
    assert_eq!(eq(false, 1), Ok(0));
    assert_eq!(eq(true, 0), Ok(0));
}

#[test]
fn test_gt() {
    let gt = jit_function! { gt(a:bool, b:bool) -> i64 => r##"
        def gt(a: bool, b: bool):
            if a > b:
                return 1
            return 0
    "## };

    assert_eq!(gt(false, false), Ok(0));
    assert_eq!(gt(true, true), Ok(0));
    assert_eq!(gt(false, true), Ok(0));
    assert_eq!(gt(true, false), Ok(1));
}

#[test]
fn test_gt_with_integers() {
    let gt = jit_function! { gt(a:i64, b:bool) -> i64 => r##"
        def gt(a: int, b: bool):
            if a > b:
                return 1
            return 0
    "## };

    assert_eq!(gt(0, false), Ok(0));
    assert_eq!(gt(1, true), Ok(0));
    assert_eq!(gt(0, true), Ok(0));
    assert_eq!(gt(1, false), Ok(1));
}

#[test]
fn test_lt() {
    let lt = jit_function! { lt(a:bool, b:bool) -> i64 => r##"
        def lt(a: bool, b: bool):
            if a < b:
                return 1
            return 0
    "## };

    assert_eq!(lt(false, false), Ok(0));
    assert_eq!(lt(true, true), Ok(0));
    assert_eq!(lt(false, true), Ok(1));
    assert_eq!(lt(true, false), Ok(0));
}

#[test]
fn test_lt_with_integers() {
    let lt = jit_function! { lt(a:i64, b:bool) -> i64 => r##"
        def lt(a: int, b: bool):
            if a < b:
                return 1
            return 0
    "## };

    assert_eq!(lt(0, false), Ok(0));
    assert_eq!(lt(1, true), Ok(0));
    assert_eq!(lt(0, true), Ok(1));
    assert_eq!(lt(1, false), Ok(0));
}

#[test]
fn test_gte() {
    let gte = jit_function! { gte(a:bool, b:bool) -> i64 => r##"
        def gte(a: bool, b: bool):
            if a >= b:
                return 1
            return 0
    "## };

    assert_eq!(gte(false, false), Ok(1));
    assert_eq!(gte(true, true), Ok(1));
    assert_eq!(gte(false, true), Ok(0));
    assert_eq!(gte(true, false), Ok(1));
}

#[test]
fn test_gte_with_integers() {
    let gte = jit_function! { gte(a:bool, b:i64) -> i64 => r##"
        def gte(a: bool, b: int):
            if a >= b:
                return 1
            return 0
    "## };

    assert_eq!(gte(false, 0), Ok(1));
    assert_eq!(gte(true, 1), Ok(1));
    assert_eq!(gte(false, 1), Ok(0));
    assert_eq!(gte(true, 0), Ok(1));
}

#[test]
fn test_lte() {
    let lte = jit_function! { lte(a:bool, b:bool) -> i64 => r##"
        def lte(a: bool, b: bool):
            if a <= b:
                return 1
            return 0
    "## };

    assert_eq!(lte(false, false), Ok(1));
    assert_eq!(lte(true, true), Ok(1));
    assert_eq!(lte(false, true), Ok(1));
    assert_eq!(lte(true, false), Ok(0));
}

#[test]
fn test_lte_with_integers() {
    let lte = jit_function! { lte(a:bool, b:i64) -> i64 => r##"
        def lte(a: bool, b: int):
            if a <= b:
                return 1
            return 0
    "## };

    assert_eq!(lte(false, 0), Ok(1));
    assert_eq!(lte(true, 1), Ok(1));
    assert_eq!(lte(false, 1), Ok(1));
    assert_eq!(lte(true, 0), Ok(0));
}
