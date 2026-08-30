#[cfg(test)]
mod tests {
    use rustpython_jit::{AbiValue, JitEngine, Outcome, Safety, StackValue};

    fn int(value: i64) -> StackValue {
        StackValue::Value(AbiValue::Int(value))
    }

    /// Overflow is not an error; it is where the interpreter stops using a
    /// machine word. The compiled code has to hand the operands back rather
    /// than wrap or trap.
    #[test]
    fn addition_deopts_on_overflow() {
        let code = jit_function! { add => r#"
def add(a: int, b: int) -> int:
    return a + b
"# };
        assert_eq!(
            code.invoke(&[1i64.into(), 2i64.into()]),
            Ok(Outcome::Returned(Some(3i64.into())))
        );
        match code.invoke(&[i64::MAX.into(), 1i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(i64::MAX), int(1)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// Subtraction is the same guard as addition, mirrored: the operands go
    /// back once the machine word can no longer hold the answer.
    #[test]
    fn subtraction_deopts_on_overflow() {
        let code = jit_function! { sub => r#"
def sub(a: int, b: int) -> int:
    return a - b
"# };
        assert_eq!(
            code.invoke(&[5i64.into(), 3i64.into()]),
            Ok(Outcome::Returned(Some(2i64.into())))
        );
        match code.invoke(&[i64::MIN.into(), 1i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(i64::MIN), int(1)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// Multiplication wraps just as readily as addition does, and needs the
    /// same guard.
    #[test]
    fn multiplication_deopts_on_overflow() {
        let code = jit_function! { mul => r#"
def mul(a: int, b: int) -> int:
    return a * b
"# };
        assert_eq!(
            code.invoke(&[3i64.into(), 4i64.into()]),
            Ok(Outcome::Returned(Some(12i64.into())))
        );
        match code.invoke(&[i64::MAX.into(), 2i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(i64::MAX), int(2)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// Negation is lowered as `0 - a`, so it overflows exactly where
    /// subtraction does. The zero is never on the interpreter's stack, so the
    /// guard records only `a`.
    #[test]
    fn negation_deopts_on_overflow() {
        let code = jit_function! { neg => r#"
def neg(a: int) -> int:
    return -a
"# };
        assert_eq!(
            code.invoke(&[5i64.into()]),
            Ok(Outcome::Returned(Some((-5i64).into())))
        );
        match code.invoke(&[i64::MIN.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(i64::MIN)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// A shift count outside `0..64` is not a machine shift at all: negative
    /// raises `ValueError`, and 64 or more is a well-defined answer the
    /// instruction cannot give. A left shift that pushes bits off the top is
    /// where the interpreter widens.
    #[test]
    fn left_shift_deopts_out_of_range_or_lossy() {
        let code = jit_function! { shl => r#"
def shl(a: int, b: int) -> int:
    return a << b
"# };
        assert_eq!(
            code.invoke(&[1i64.into(), 2i64.into()]),
            Ok(Outcome::Returned(Some(4i64.into())))
        );
        match code.invoke(&[1i64.into(), 64i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(1), int(64)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
        match code.invoke(&[1i64.into(), (-1i64).into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(1), int(-1)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
        match code.invoke(&[1i64.into(), 63i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(1), int(63)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// A negative count and a count of 64 or more are out of range for the
    /// machine instruction the same way they are for a left shift.
    #[test]
    fn right_shift_deopts_out_of_range() {
        let code = jit_function! { shr => r#"
def shr(a: int, b: int) -> int:
    return a >> b
"# };
        assert_eq!(
            code.invoke(&[8i64.into(), 1i64.into()]),
            Ok(Outcome::Returned(Some(4i64.into())))
        );
        match code.invoke(&[8i64.into(), 64i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(8), int(64)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
        match code.invoke(&[8i64.into(), (-1i64).into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(8), int(-1)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// A zero divisor has no machine answer: `sdiv`/`srem` trap where Python
    /// raises ZeroDivisionError. `//` and `%` share the same guard.
    #[test]
    fn floor_divide_and_remainder_deopt_on_zero_divisor() {
        let div = jit_function! { div => r#"
def div(a: int, b: int) -> int:
    return a // b
"# };
        assert_eq!(
            div.invoke(&[7i64.into(), 2i64.into()]),
            Ok(Outcome::Returned(Some(3i64.into())))
        );
        match div.invoke(&[7i64.into(), 0i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(7), int(0)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }

        let rem = jit_function! { rem => r#"
def rem(a: int, b: int) -> int:
    return a % b
"# };
        match rem.invoke(&[7i64.into(), 0i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(7), int(0)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// `i64::MIN // -1`'s quotient is one past the top of the range - there
    /// is no 64-bit answer, so it deoptimizes ahead of the `sdiv` that would
    /// otherwise trap.
    #[test]
    fn floor_divide_deopts_on_i64_min_over_negative_one() {
        let code = jit_function! { div => r#"
def div(a: int, b: int) -> int:
    return a // b
"# };
        match code.invoke(&[i64::MIN.into(), (-1i64).into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(i64::MIN), int(-1)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// A zero divisor raises ZeroDivisionError; there is no float to hand
    /// back for it.
    #[test]
    fn true_divide_deopts_on_zero_divisor() {
        let code = jit_function! { div => r#"
def div(a: int, b: int) -> float:
    return a / b
"# };
        assert_eq!(
            code.invoke(&[7i64.into(), 2i64.into()]),
            Ok(Outcome::Returned(Some(3.5f64.into())))
        );
        match code.invoke(&[7i64.into(), 0i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(7), int(0)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// `int.__truediv__` is correctly rounded, but converting both operands
    /// to `f64` and dividing rounds twice. That can be a ulp out as soon as
    /// either conversion is inexact, which is exactly when an operand does
    /// not fit a double's 53-bit significand.
    #[test]
    fn true_divide_deopts_on_wide_operand() {
        let code = jit_function! { div => r#"
def div(a: int, b: int) -> float:
    return a / b
"# };
        assert_eq!(
            code.invoke(&[((1i64 << 53) - 1).into(), 1i64.into()]),
            Ok(Outcome::Returned(Some((((1i64 << 53) - 1) as f64).into())))
        );
        match code.invoke(&[(1i64 << 53).into(), 1i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(1i64 << 53), int(1)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// `iabs` of `i64::MIN` is `i64::MIN` again, which an unsigned comparison
    /// reads as far above `1 << 53` - so it takes the wide-operand path too.
    #[test]
    fn true_divide_deopts_on_i64_min_operand() {
        let code = jit_function! { div => r#"
def div(a: int, b: int) -> float:
    return a / b
"# };
        match code.invoke(&[i64::MIN.into(), 1i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(i64::MIN), int(1)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// A guard reports every live local, and reports a local that has not been
    /// assigned on this path as unbound rather than inventing a value for it.
    #[test]
    fn a_guard_reports_the_live_state() {
        let code = jit_function! { f => r#"
def f(a: int, b: int, c: bool) -> int:
    if c:
        d = 5
    return a + b
"# };
        match code.invoke(&[i64::MAX.into(), 1i64.into(), false.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.locals[0], Some(AbiValue::Int(i64::MAX)));
                assert_eq!(state.locals[1], Some(AbiValue::Int(1)));
                assert_eq!(state.locals[2], Some(AbiValue::Bool(false)));
                // `d` was declared by the store inside the branch, but that
                // branch did not run.
                assert_eq!(state.locals[3], None);
                assert_eq!(state.stack, vec![int(i64::MAX), int(1)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
        // The same function, on the path that does assign `d`.
        match code.invoke(&[i64::MAX.into(), 1i64.into(), true.into()]) {
            Ok(Outcome::Deopt(state)) => assert_eq!(state.locals[3], Some(AbiValue::Int(5))),
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// A self-call leaves its callable and a null underneath the arguments
    /// being evaluated. Neither has a slot in the buffer, but both are the same
    /// on every path that reaches the guard, so the site describes them and the
    /// function stays compilable.
    #[test]
    fn a_guard_under_a_self_call_describes_the_callable() {
        let engine = JitEngine::new();
        let f = py_function_def! { countdown => r#"
def countdown(a: int, b: int) -> int:
    if a < 0:
        return b
    return countdown(a + b, b)
"# };
        let code = f
            .compile_on(&engine, Safety::Permissive)
            .expect("a guard below a self-call must not stop the function compiling");
        match code.invoke(&[i64::MAX.into(), 1i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(
                    state.stack,
                    vec![StackValue::Callee, StackValue::Null, int(i64::MAX), int(1),]
                );
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// A nested frame that gives up returns a filler in place of a result. The
    /// caller has to stop rather than compute on it: the record the callee
    /// wrote is the one the interpreter needs.
    #[test]
    fn a_caller_stops_when_a_nested_frame_gives_up() {
        let engine = JitEngine::new();
        let f = py_function_def! { blow => r#"
def blow(n: int) -> int:
    if n == 0:
        return 4611686018427387904
    return blow(n - 1) + blow(n - 1)
"# };
        let code = f
            .compile_on(&engine, Safety::Permissive)
            .expect("should compile");
        assert_eq!(
            code.invoke(&[0i64.into()]),
            Ok(Outcome::Returned(Some(4611686018427387904i64.into())))
        );
        // `blow(1)` overflows. `blow(2)` reaches that overflow one frame down,
        // so the guard that fires is the inner frame's and its record is what
        // comes back.
        for n in [1i64, 2] {
            match code.invoke(&[n.into()]) {
                Ok(Outcome::Deopt(state)) => assert_eq!(
                    state.stack,
                    vec![int(4611686018427387904), int(4611686018427387904)],
                    "n = {n}"
                ),
                other => panic!("expected a deopt for n = {n}, got {other:?}"),
            }
        }
    }
}
