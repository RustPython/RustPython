#[cfg(test)]
mod tests {
    use rustpython_jit::{JitEngine, Outcome, Safety};

    /// Every operation whose machine code can trap or wrap. There is no trap
    /// handler, so a trap kills the process instead of raising.
    /// Assert Strict rejects the function *and* that Permissive still accepts
    /// it, so the test cannot pass because of an unrelated compile failure.
    macro_rules! assert_strict_rejects {
        ($name:ident => $src:expr) => {{
            let engine = JitEngine::new();
            let f = py_function_def!($name => $src);
            assert!(
                f.compile_on(&engine, Safety::Strict).is_err(),
                concat!(stringify!($name), " should not compile under Strict")
            );
            f.compile_on(&engine, Safety::Permissive).expect(concat!(
                stringify!($name),
                " is only meant to be rejected for being unsafe, but Permissive                  cannot compile it either"
            ));
        }};
    }

    macro_rules! assert_accepted {
        ($safety:expr, $name:ident => $src:expr) => {{
            let engine = JitEngine::new();
            let f = py_function_def!($name => $src);
            f.compile_on(&engine, $safety)
                .expect(concat!(stringify!($name), " should compile"))
        }};
    }

    #[test]
    fn strict_rejects_int_add() {
        assert_strict_rejects!(add => r#"
def add(a: int, b: int) -> int:
    return a + b
"#);
    }

    #[test]
    fn strict_rejects_int_multiply() {
        assert_strict_rejects!(mul => r#"
def mul(a: int, b: int) -> int:
    return a * b
"#);
    }

    #[test]
    fn strict_rejects_int_floor_divide() {
        assert_strict_rejects!(fdiv => r#"
def fdiv(a: int, b: int) -> int:
    return a // b
"#);
    }

    #[test]
    fn strict_rejects_int_true_divide() {
        assert_strict_rejects!(true_divide => r#"
def true_divide(a: int, b: int) -> float:
    return a / b
"#);
    }

    #[test]
    fn strict_rejects_int_remainder() {
        assert_strict_rejects!(rem => r#"
def rem(a: int, b: int) -> int:
    return a % b
"#);
    }

    #[test]
    fn strict_rejects_int_power() {
        assert_strict_rejects!(pow => r#"
def pow(a: int, b: int) -> int:
    return a ** b
"#);
    }

    #[test]
    fn strict_rejects_int_shift() {
        assert_strict_rejects!(shift => r#"
def shift(a: int, b: int) -> int:
    return a << b
"#);
    }

    #[test]
    #[ignore = "Task 7 makes Strict accept this"]
    fn strict_rejects_int_negate() {
        assert_strict_rejects!(neg => r#"
def neg(a: int) -> int:
    return -a
"#);
    }

    /// `1.0 / 0.0` raises ZeroDivisionError; `fdiv` returns inf.
    #[test]
    fn strict_rejects_float_divide() {
        assert_strict_rejects!(fdiv => r#"
def fdiv(a: float, b: float) -> float:
    return a / b
"#);
    }

    /// `(-1.0) ** 0.5` is complex in Python and `0.0 ** -1.0` raises.
    #[test]
    fn strict_rejects_float_power() {
        assert_strict_rejects!(float_power => r#"
def float_power(a: float, b: float) -> float:
    return a ** b
"#);
    }

    #[test]
    fn strict_rejects_mixed_divide() {
        assert_strict_rejects!(mixed => r#"
def mixed(a: int, b: float) -> float:
    return a / b
"#);
    }

    /// Bitwise operations on two machine integers cannot leave the range.
    #[test]
    fn strict_allows_int_bitwise() {
        let code = assert_accepted!(Safety::Strict, band => r#"
def band(a: int, b: int) -> int:
    return a & b
"#);
        assert_eq!(
            code.invoke(&[6i64.into(), 3i64.into()]),
            Ok(Outcome::Returned(Some(2i64.into())))
        );
    }

    #[test]
    fn strict_allows_int_comparison() {
        let code = assert_accepted!(Safety::Strict, lt => r#"
def lt(a: int, b: int) -> bool:
    return a < b
"#);
        assert_eq!(
            code.invoke(&[1i64.into(), 2i64.into()]),
            Ok(Outcome::Returned(Some(true.into())))
        );
    }

    #[test]
    fn strict_allows_float_add_and_multiply() {
        let code = assert_accepted!(Safety::Strict, poly => r#"
def poly(a: float, b: float) -> float:
    return a * b + a - b
"#);
        assert_eq!(
            code.invoke(&[2.0f64.into(), 3.0f64.into()]),
            Ok(Outcome::Returned(Some(5.0f64.into())))
        );
    }

    /// Mixing an int into float addition converts exactly the way the
    /// interpreter does, so it stays available under Strict.
    #[test]
    fn strict_allows_mixed_add() {
        let code = assert_accepted!(Safety::Strict, mixed => r#"
def mixed(a: int, b: float) -> float:
    return a + b
"#);
        assert_eq!(
            code.invoke(&[2i64.into(), 0.5f64.into()]),
            Ok(Outcome::Returned(Some(2.5f64.into())))
        );
    }

    /// The self-reference resolves by name, but the interpreter re-reads the
    /// global on every call, so a rebound name would make them disagree.
    #[test]
    fn strict_rejects_self_recursion() {
        assert_strict_rejects!(countdown => r#"
def countdown(a: float) -> float:
    if a > 0.0:
        return countdown(a - 1.0)
    return a
"#);
    }

    #[test]
    fn permissive_still_compiles_int_arithmetic() {
        let code = assert_accepted!(Safety::Permissive, add => r#"
def add(a: int, b: int) -> int:
    return a + b
"#);
        assert_eq!(
            code.invoke(&[3i64.into(), 4i64.into()]),
            Ok(Outcome::Returned(Some(7i64.into())))
        );
    }
}
