#[cfg(test)]
mod tests {
    use rustpython_jit::{JitEngine, Outcome, Safety};

    /// Assert Strict rejects the function while Permissive still accepts it,
    /// so the test cannot pass because of an unrelated compile failure.
    macro_rules! assert_strict_rejects {
        ($name:ident => $src:expr) => {{
            let engine = JitEngine::new(None);
            let f = py_function_def!($name => $src);
            assert!(
                f.compile_on(&engine, Safety::Strict).is_err(),
                concat!(stringify!($name), " should not compile under Strict")
            );
            f.compile_on(&engine, Safety::Permissive).expect(concat!(
                stringify!($name),
                " is only meant to be rejected for being unsafe, but Permissive cannot compile it either"
            ));
        }};
    }

    macro_rules! assert_accepted {
        ($safety:expr, $name:ident => $src:expr) => {{
            let engine = JitEngine::new(None);
            let f = py_function_def!($name => $src);
            f.compile_on(&engine, $safety)
                .expect(concat!(stringify!($name), " should compile"))
        }};
    }

    /// The operation used to be refused under Strict because its machine
    /// code could trap or wrap. Both are guarded by a deopt now, so Strict
    /// compiles the function and, given the input that used to be unsafe,
    /// hands the operands back rather than trapping, wrapping, or otherwise
    /// answering wrongly. An optional fourth and fifth argument also pin an
    /// ordinary input's answer, so a guard that regressed to an
    /// unconditional deopt could not leave every test in this file green.
    macro_rules! assert_strict_deopts {
        ($name:ident => $src:expr, $bad:expr) => {{
            let code = assert_accepted!(Safety::Strict, $name => $src);
            match code.invoke(&$bad) {
                Ok(Outcome::Deopt(_)) => {}
                other => panic!(
                    "{} expected a deopt under Strict, got {other:?}",
                    stringify!($name)
                ),
            }
        }};
        ($name:ident => $src:expr, $bad:expr, $good:expr, $expected:expr) => {{
            let code = assert_accepted!(Safety::Strict, $name => $src);
            assert_eq!(
                code.invoke(&$good),
                Ok(Outcome::Returned(Some($expected.into()))),
                "{} expected the ordinary input to still answer",
                stringify!($name)
            );
            match code.invoke(&$bad) {
                Ok(Outcome::Deopt(_)) => {}
                other => panic!(
                    "{} expected a deopt under Strict, got {other:?}",
                    stringify!($name)
                ),
            }
        }};
    }

    #[test]
    fn strict_compiles_int_add() {
        assert_strict_deopts!(add => r#"
def add(a: int, b: int) -> int:
    return a + b
"#, [i64::MAX.into(), 1i64.into()], [3i64.into(), 4i64.into()], 7i64);
    }

    #[test]
    fn strict_compiles_int_multiply() {
        assert_strict_deopts!(mul => r#"
def mul(a: int, b: int) -> int:
    return a * b
"#, [i64::MAX.into(), 2i64.into()]);
    }

    #[test]
    fn strict_compiles_int_floor_divide() {
        assert_strict_deopts!(fdiv => r#"
def fdiv(a: int, b: int) -> int:
    return a // b
"#, [7i64.into(), 0i64.into()]);
    }

    #[test]
    fn strict_compiles_int_true_divide() {
        assert_strict_deopts!(true_divide => r#"
def true_divide(a: int, b: int) -> float:
    return a / b
"#, [7i64.into(), 0i64.into()]);
    }

    #[test]
    fn strict_compiles_int_remainder() {
        assert_strict_deopts!(rem => r#"
def rem(a: int, b: int) -> int:
    return a % b
"#, [7i64.into(), 0i64.into()]);
    }

    #[test]
    fn strict_compiles_int_power() {
        assert_strict_deopts!(pow => r#"
def pow(a: int, b: int) -> int:
    return a ** b
"#, [2i64.into(), 64i64.into()]);
    }

    #[test]
    fn strict_compiles_int_shift() {
        assert_strict_deopts!(shift => r#"
def shift(a: int, b: int) -> int:
    return a << b
"#, [1i64.into(), 64i64.into()]);
    }

    #[test]
    fn strict_compiles_int_negate() {
        assert_strict_deopts!(neg => r#"
def neg(a: int) -> int:
    return -a
"#, [i64::MIN.into()]);
    }

    /// `1.0 / 0.0` raises ZeroDivisionError; a bare `fdiv` would return inf.
    #[test]
    fn strict_compiles_float_divide() {
        assert_strict_deopts!(fdiv => r#"
def fdiv(a: float, b: float) -> float:
    return a / b
"#, [1.0f64.into(), 0.0f64.into()]);
    }

    /// `(-8.0) ** 0.5` is complex in Python, which a compiled `**` cannot
    /// produce.
    #[test]
    fn strict_compiles_float_power() {
        assert_strict_deopts!(float_power => r#"
def float_power(a: float, b: float) -> float:
    return a ** b
"#, [(-8.0f64).into(), 0.5f64.into()]);
    }

    #[test]
    fn strict_compiles_mixed_divide() {
        assert_strict_deopts!(mixed => r#"
def mixed(a: int, b: float) -> float:
    return a / b
"#, [1i64.into(), 0.0f64.into()]);
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
    /// This is the one place Strict and Permissive still differ.
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
