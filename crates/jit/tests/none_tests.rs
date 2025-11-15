#[test]
fn test_not() {
    let not_ = jit_function! { not_(x: i64) -> bool => r##"
        def not_(x: int):
            return not None
    "## };

    assert_eq!(not_(0), Ok(true));
}

#[test]
fn test_if_not() {
    let if_not = jit_function! { if_not(x: i64) -> i64 => r##"
        def if_not(x: int):
            if not None:
                return 1
            else:
                return 0

            return -1
    "## };

    assert_eq!(if_not(0), Ok(1));
}
