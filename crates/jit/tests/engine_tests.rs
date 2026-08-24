#[cfg(test)]
mod tests {
    use rustpython_jit::{JitEngine, Safety};

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
            Ok(Some(3i64.into()))
        );
        assert_eq!(
            second.invoke(&[3i64.into(), 4i64.into()]),
            Ok(Some(4i64.into()))
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
            Ok(Some(7i64.into()))
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
        assert_eq!(code.invoke(&[7i64.into()]), Ok(Some(7i64.into())));
    }
}
