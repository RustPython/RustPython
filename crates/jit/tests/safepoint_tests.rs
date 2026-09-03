#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicU8, Ordering};
    use rustpython_jit::{AbiValue, JitEngine, Outcome, Safety};

    /// A loop polls the word it was compiled against every time round, and
    /// leaves as soon as it reads anything but zero. The record it leaves says
    /// where the loop had got to, so the interpreter picks the same iteration
    /// back up rather than starting the function again.
    ///
    /// Each test owns its word: the engines are separate, and a shared one
    /// would leak the trip into whichever test happened to run beside it.
    #[test]
    fn a_loop_leaves_when_the_word_it_polls_is_set() {
        static WORD: AtomicU8 = AtomicU8::new(0);
        let engine = JitEngine::new(Some(&WORD));
        let f = py_function_def! { spin => r#"
def spin(n: int) -> int:
    i = 0
    while i < n:
        i = i + 1
    return i
"# };
        let code = f
            .compile_on(&engine, Safety::Strict)
            .expect("should compile");
        assert_eq!(
            code.invoke(&[3i64.into()]),
            Ok(Outcome::Returned(Some(3i64.into())))
        );

        WORD.store(1, Ordering::Release);
        match code.invoke(&[3i64.into()]) {
            Ok(Outcome::Interrupted(Some(state))) => {
                // One pass through the body, then the jump back polls.
                assert_eq!(state.locals[0], Some(AbiValue::Int(3)));
                assert_eq!(state.locals[1], Some(AbiValue::Int(1)));
                assert!(state.stack.is_empty(), "{:?}", state.stack);
            }
            other => panic!("expected an interruption, got {other:?}"),
        }
    }

    /// The word is read on every iteration, not once on the way in: a loop
    /// already running has to notice a word set while it runs. Nothing else
    /// here proves the load survives into the loop body rather than being
    /// hoisted out of it.
    #[test]
    fn a_running_loop_notices_the_word_being_set() {
        static WORD: AtomicU8 = AtomicU8::new(0);
        let engine = JitEngine::new(Some(&WORD));
        let f = py_function_def! { spin => r#"
def spin(n: int) -> int:
    i = 0
    while i < n:
        i = i + 1
    return i
"# };
        let code = f
            .compile_on(&engine, Safety::Strict)
            .expect("should compile");

        // Long enough that the setter lands somewhere in the middle of it, and
        // small enough that the test still ends if it does not.
        let iterations = 1i64 << 32;
        let setter = std::thread::spawn(|| {
            std::thread::sleep(core::time::Duration::from_millis(20));
            WORD.store(1, Ordering::Release);
        });
        let outcome = code.invoke(&[iterations.into()]);
        setter.join().expect("the setter must not panic");

        match outcome {
            Ok(Outcome::Interrupted(Some(state))) => {
                let Some(AbiValue::Int(reached)) = state.locals[1] else {
                    panic!("the counter must come back as an int: {state:?}");
                };
                assert!(
                    (0..iterations).contains(&reached),
                    "left mid-loop, not at either end: {reached}"
                );
            }
            other => panic!("expected an interruption, got {other:?}"),
        }
        WORD.store(0, Ordering::Release);
    }

    /// A function with no loop can only run for as long as its own body, so it
    /// polls nowhere and runs to its end however the word reads.
    #[test]
    fn a_straight_line_function_does_not_poll() {
        static WORD: AtomicU8 = AtomicU8::new(1);
        let engine = JitEngine::new(Some(&WORD));
        let f = py_function_def! { add => r#"
def add(a: int, b: int) -> int:
    return a + b
"# };
        let code = f
            .compile_on(&engine, Safety::Strict)
            .expect("should compile");
        assert_eq!(
            code.invoke(&[2i64.into(), 3i64.into()]),
            Ok(Outcome::Returned(Some(5i64.into())))
        );
    }

    /// Compiled without a word there is nothing to poll, and the loop runs to
    /// completion. This is what an engine with no interpreter behind it gets.
    #[test]
    fn a_loop_compiled_against_no_word_runs_to_the_end() {
        let engine = JitEngine::new(None);
        let f = py_function_def! { spin => r#"
def spin(n: int) -> int:
    i = 0
    while i < n:
        i = i + 1
    return i
"# };
        let code = f
            .compile_on(&engine, Safety::Strict)
            .expect("should compile");
        assert_eq!(
            code.invoke(&[100i64.into()]),
            Ok(Outcome::Returned(Some(100i64.into())))
        );
    }
}
