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
