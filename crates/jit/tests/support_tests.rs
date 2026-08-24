#[cfg(test)]
mod tests {
    use rustpython_jit::supports_code;

    macro_rules! assert_supported {
        ($name:ident => $src:expr) => {{
            let f = py_function_def!($name => $src);
            assert!(
                supports_code(f.code()),
                concat!(stringify!($name), " should pass the pre-filter")
            );
        }};
    }

    macro_rules! assert_unsupported {
        ($name:ident => $src:expr) => {{
            let f = py_function_def!($name => $src);
            assert!(
                !supports_code(f.code()),
                concat!(stringify!($name), " should be rejected by the pre-filter")
            );
        }};
    }

    #[test]
    fn plain_arithmetic_is_supported() {
        assert_supported!(add => r#"
def add(a: int, b: int) -> int:
    return a + b
"#);
    }

    #[test]
    fn branches_and_loops_are_supported() {
        assert_supported!(count => r#"
def count(n: int) -> int:
    total = 0
    while n > 0:
        if n > 5:
            total = total + 1
        n = n - 1
    return total
"#);
    }

    #[test]
    fn varargs_are_rejected() {
        assert_unsupported!(va => r#"
def va(*args) -> int:
    return 1
"#);
    }

    #[test]
    fn varkeywords_are_rejected() {
        assert_unsupported!(vk => r#"
def vk(**kwargs) -> int:
    return 1
"#);
    }

    #[test]
    fn generators_are_rejected() {
        assert_unsupported!(gen => r#"
def gen(n: int):
    yield n
"#);
    }

    #[test]
    fn containers_are_rejected() {
        assert_unsupported!(indexing => r#"
def indexing(a: int) -> int:
    return [a][0]
"#);
    }

    #[test]
    fn attribute_access_is_rejected() {
        assert_unsupported!(attr => r#"
def attr(a: int) -> int:
    return a.bit_length()
"#);
    }

    #[test]
    fn exception_handling_is_rejected() {
        assert_unsupported!(guarded => r#"
def guarded(a: int) -> int:
    try:
        return a
    except ValueError:
        return 0
"#);
    }
}
