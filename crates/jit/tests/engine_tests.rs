#[cfg(test)]
mod tests {
    use rustpython_jit::{JitCompileError, JitEngine, Outcome, Safety};

    /// Two Python functions can share `obj_name`, so the module-level symbol has to
    /// be made unique before they can live in one engine.
    #[test]
    fn same_name_functions_coexist() {
        let engine = JitEngine::new();
        let first = py_function_def!(foo => r#"
    def foo(a: int, b: int) -> int:
        return a
    "#);
        let second = py_function_def!(foo => r#"
    def foo(a: int, b: int) -> int:
        return b
    "#);

        let first = first
            .compile_on(&engine, Safety::Permissive)
            .expect("first compile");
        let second = second
            .compile_on(&engine, Safety::Permissive)
            .expect("second compile of the same name");

        assert_eq!(
            first.invoke(&[3i64.into(), 4i64.into()]),
            Ok(Outcome::Returned(Some(3i64.into())))
        );
        assert_eq!(
            second.invoke(&[3i64.into(), 4i64.into()]),
            Ok(Outcome::Returned(Some(4i64.into())))
        );
    }

    /// A rejected function must not leave half-built state in the shared context.
    #[test]
    fn failed_compile_does_not_poison_engine() {
        let engine = JitEngine::new();
        let unsupported = py_function_def!(unsupported => r#"
    def unsupported(a: int) -> int:
        return [a]
    "#);
        assert!(unsupported.compile_on(&engine, Safety::Permissive).is_err());

        let good = py_function_def!(good => r#"
    def good(a: int, b: int) -> int:
        return a + b
    "#);
        let good = good
            .compile_on(&engine, Safety::Permissive)
            .expect("engine still usable after a rejected function");
        assert_eq!(
            good.invoke(&[3i64.into(), 4i64.into()]),
            Ok(Outcome::Returned(Some(7i64.into())))
        );
    }

    /// Compiled code outliving the caller's handle on the engine must still run.
    #[test]
    fn compiled_code_keeps_engine_alive() {
        let code = {
            let engine = JitEngine::new();
            let f = py_function_def!(f => r#"
    def f(a: int) -> int:
        return a
    "#);
            f.compile_on(&engine, Safety::Permissive).expect("compile")
        };
        assert_eq!(
            code.invoke(&[7i64.into()]),
            Ok(Outcome::Returned(Some(7i64.into())))
        );
    }

    /// Every parameter reaches the compiled body from its own slot, whatever
    /// the types around it are.
    #[test]
    fn mixed_signature_round_trip() {
        let pick_int = jit_function! { pick_int(a: i64, b: f64, c: bool, d: f64, e: i64) -> i64 => r#"
    def pick_int(a: int, b: float, c: bool, d: float, e: int) -> int:
        if c:
            return a
        return e
    "# };
        assert_eq!(pick_int(1, 2.5, true, 4.5, 5), Ok(1));
        assert_eq!(pick_int(1, 2.5, false, 4.5, 5), Ok(5));

        let pick_float = jit_function! { pick_float(a: i64, b: f64, c: bool, d: f64, e: i64) -> f64 => r#"
    def pick_float(a: int, b: float, c: bool, d: float, e: int) -> float:
        if c:
            return b
        return d
    "# };
        assert_eq!(pick_float(1, 2.5, true, 4.5, 5), Ok(2.5));
        assert_eq!(pick_float(1, 2.5, false, 4.5, 5), Ok(4.5));
    }

    /// Arguments travel in a fixed-size buffer, so a function wider than the
    /// buffer is turned down rather than compiled into an overrun.
    #[test]
    fn parameters_beyond_the_buffer_are_rejected() {
        let engine = JitEngine::new();
        let widest = py_function_def!(widest => r#"
    def widest(a0: int, a1: int, a2: int, a3: int, a4: int, a5: int, a6: int, a7: int, a8: int, a9: int, a10: int, a11: int, a12: int, a13: int, a14: int, a15: int) -> int:
        return a15
    "#);
        let widest = widest
            .compile_on(&engine, Safety::Permissive)
            .expect("a function that fills the buffer still compiles");
        let args: Vec<_> = (0..16).map(|i| i64::from(i).into()).collect();
        assert_eq!(
            widest.invoke(&args),
            Ok(Outcome::Returned(Some(15i64.into())))
        );

        let too_wide = py_function_def!(too_wide => r#"
    def too_wide(a0: int, a1: int, a2: int, a3: int, a4: int, a5: int, a6: int, a7: int, a8: int, a9: int, a10: int, a11: int, a12: int, a13: int, a14: int, a15: int, a16: int) -> int:
        return a16
    "#);
        assert!(matches!(
            too_wide.compile_on(&engine, Safety::Permissive),
            Err(JitCompileError::NotSupported)
        ));
    }
}
