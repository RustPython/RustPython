#[cfg(test)]
mod tests {
    use rustpython_jit::{AbiValue, JitEngine, Outcome, Safety, StackValue};

    fn int(value: i64) -> StackValue {
        StackValue::Value(AbiValue::Int(value))
    }

    fn float(value: f64) -> StackValue {
        StackValue::Value(AbiValue::Float(value))
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
    /// otherwise trap. The guard is shared with the remainder, so
    /// `i64::MIN % -1` deopts too even though `0` is a perfectly good
    /// answer for it.
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

        let rem = jit_function! { rem => r#"
def rem(a: int, b: int) -> int:
    return a % b
"# };
        match rem.invoke(&[i64::MIN.into(), (-1i64).into()]) {
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
        // `1 << 53` itself converts to a double exactly, so it stays compiled.
        assert_eq!(
            code.invoke(&[(1i64 << 53).into(), 1i64.into()]),
            Ok(Outcome::Returned(Some(((1i64 << 53) as f64).into())))
        );
        match code.invoke(&[((1i64 << 53) + 1).into(), 1i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int((1i64 << 53) + 1), int(1)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
        // The guard checks both operands - a wide divisor deopts as readily
        // as a wide dividend.
        match code.invoke(&[1i64.into(), ((1i64 << 53) + 1).into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(1), int((1i64 << 53) + 1)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
        // `iabs` cannot negate `i64::MIN`; the unsigned comparison still
        // places it past the bound.
        match code.invoke(&[i64::MIN.into(), 1i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(i64::MIN), int(1)]);
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

    /// A negative exponent makes `**` a float, and the loop only computes
    /// integers.
    #[test]
    fn power_deopts_on_negative_exponent() {
        let code = jit_function! { pow => r#"
def pow(a: int, b: int) -> int:
    return a ** b
"# };
        assert_eq!(
            code.invoke(&[2i64.into(), 2i64.into()]),
            Ok(Outcome::Returned(Some(4i64.into())))
        );
        match code.invoke(&[2i64.into(), (-2i64).into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(2), int(-2)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// `2 ** 64` does not fit an i64: the loop's final squaring carries into
    /// the 65th bit even though every earlier iteration stayed in range.
    #[test]
    fn power_deopts_when_the_answer_does_not_fit() {
        let code = jit_function! { pow => r#"
def pow(a: int, b: int) -> int:
    return a ** b
"# };
        match code.invoke(&[2i64.into(), 64i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![int(2), int(64)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// A zero divisor raises ZeroDivisionError; `fdiv` would otherwise return
    /// an infinity. `fcmp Equal` against `0.0` catches `-0.0` too, which
    /// raises the same way.
    #[test]
    fn true_divide_deopts_on_float_zero_divisor() {
        let code = jit_function! { div => r#"
def div(a: float, b: float) -> float:
    return a / b
"# };
        assert_eq!(
            code.invoke(&[4.0f64.into(), 2.0f64.into()]),
            Ok(Outcome::Returned(Some(2.0f64.into())))
        );
        for divisor in [0.0f64, -0.0f64] {
            match code.invoke(&[1.0f64.into(), divisor.into()]) {
                Ok(Outcome::Deopt(state)) => {
                    assert_eq!(state.stack, vec![float(1.0), float(divisor)]);
                }
                other => panic!("expected a deopt for {divisor}, got {other:?}"),
            }
        }
    }

    /// The mixed int/float arm has its own `fdiv`, reached whenever exactly
    /// one operand is already a float, and needs the same guard regardless of
    /// which side that is - including a `-0.0` divisor, the same as the
    /// float/float arm above.
    #[test]
    fn true_divide_deopts_on_mixed_zero_divisor() {
        let int_over_float = jit_function! { div => r#"
def div(a: int, b: float) -> float:
    return a / b
"# };
        for divisor in [0.0f64, -0.0f64] {
            match int_over_float.invoke(&[1i64.into(), divisor.into()]) {
                Ok(Outcome::Deopt(state)) => {
                    assert_eq!(state.stack, vec![int(1), float(divisor)]);
                }
                other => panic!("expected a deopt for {divisor}, got {other:?}"),
            }
        }

        let float_over_int = jit_function! { div => r#"
def div(a: float, b: int) -> float:
    return a / b
"# };
        match float_over_int.invoke(&[1.0f64.into(), 0i64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![float(1.0), int(0)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }

    /// `0.0 ** negative` raises rather than returning an infinity. The
    /// exponent is read by its value, so `-0.0` is not one of them - see
    /// `basic_power` in float_tests.rs for the answer it gets instead.
    #[test]
    fn float_power_deopts_on_zero_base_negative_exponent() {
        let code = jit_function! { pow => r#"
def pow(a: float, b: float) -> float:
    return a ** b
"# };
        // A `-0.0` base is a zero base like any other.
        for base in [0.0f64, -0.0f64] {
            match code.invoke(&[base.into(), (-1.0f64).into()]) {
                Ok(Outcome::Deopt(state)) => {
                    assert_eq!(state.stack, vec![float(base), float(-1.0)]);
                }
                other => panic!("expected a deopt for {base} ** -1.0, got {other:?}"),
            }
        }
    }

    /// A negative base raised to a fractional power is complex. The base is
    /// read by its value too, so `-0.0` is not one of those either.
    #[test]
    fn float_power_deopts_on_negative_base_fractional_exponent() {
        let code = jit_function! { pow => r#"
def pow(a: float, b: float) -> float:
    return a ** b
"# };
        match code.invoke(&[(-8.0f64).into(), 0.5f64.into()]) {
            Ok(Outcome::Deopt(state)) => {
                assert_eq!(state.stack, vec![float(-8.0), float(0.5)]);
            }
            other => panic!("expected a deopt for -8.0 ** 0.5, got {other:?}"),
        }
    }

    /// A finite base and exponent whose true power overflows a double raises
    /// OverflowError rather than saturating to an infinity - unlike
    /// `inf ** 2.0`, which correctly keeps returning `inf` and must not
    /// deopt here. `1e100 ** 1e50` used to kill the process outright: an old
    /// double–double implementation rounded `b * ln|a|` to an i64 with a
    /// `fcvt_to_sint` that trapped once the product left i64 range, and
    /// cranelift traps have no handler. Calling `f64::powf` directly has no
    /// such trap, so it deoptimizes here like every other overflow shape.
    #[test]
    fn float_power_deopts_on_finite_base_overflow() {
        let code = jit_function! { pow => r#"
def pow(a: float, b: float) -> float:
    return a ** b
"# };
        assert_eq!(
            code.invoke(&[f64::INFINITY.into(), 2.0f64.into()]),
            Ok(Outcome::Returned(Some(f64::INFINITY.into())))
        );
        for (a, b) in [
            (1e308f64, 2.0f64),
            (2.0f64, 1e300f64),
            (-2.0f64, 1e300f64),
            (1e100f64, 1e50f64),
            (2.0f64, 1024.0f64),
            (1e-308f64, -2.0f64),
            (1e-308f64, -320.0f64),
            (1e-100f64, -320.0f64),
            (1e100f64, 4.0f64),
        ] {
            match code.invoke(&[a.into(), b.into()]) {
                Ok(Outcome::Deopt(state)) => {
                    assert_eq!(state.stack, vec![float(a), float(b)], "{a} ** {b}");
                }
                other => panic!("expected a deopt for {a} ** {b}, got {other:?}"),
            }
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
        let engine = JitEngine::new(None);
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
    /// caller has to stop rather than compute on it.
    ///
    /// Which of the two answers comes back depends on whose guard fired.
    /// `blow(1)` overflows on its own addition, so its record describes the
    /// frame that is asking for it. `blow(2)` reaches that overflow one frame
    /// down, and the record standing in the buffer belongs to that frame - the
    /// two frames run the same code, so every type in it lines up and a resume
    /// would silently continue the outer frame from the inner frame's offset.
    /// It has to come back as a restart instead.
    #[test]
    fn a_caller_stops_when_a_nested_frame_gives_up() {
        let engine = JitEngine::new(None);
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
        match code.invoke(&[1i64.into()]) {
            Ok(Outcome::Deopt(state)) => assert_eq!(
                state.stack,
                vec![int(4611686018427387904), int(4611686018427387904)]
            ),
            other => panic!("expected a deopt, got {other:?}"),
        }
        assert_eq!(code.invoke(&[2i64.into()]), Ok(Outcome::Restart));
    }

    /// A guard lists the locals the compiler had seen where it was lowered,
    /// which a backward jump can leave short: `extra` is stored further down
    /// the loop body than the guard on the sum, yet it is bound by the time
    /// that guard fires on a later iteration. Such a site cannot describe the
    /// frame, so it asks for a restart rather than resuming without it.
    ///
    /// `extra` is never read, and could not be: a read the compiler cannot
    /// prove bound is a `LoadFastCheck`, which has no lowering, so a function
    /// that would observe the drop that way does not compile. What sees it is
    /// anything reading the frame's fastlocals other than a `LoadFast` -
    /// `f_locals`, a tracer, a debugger - which is why the snippet covering
    /// this end to end goes through a traceback.
    #[test]
    fn a_site_that_cannot_describe_every_local_restarts() {
        let code = jit_function! { late => r#"
def late(n: int, step: int) -> int:
    total = 0
    while n > 0:
        total = total + n * step
        if n == 5:
            extra = 1
        n = n - 1
    return total
"# };
        assert_eq!(
            code.invoke(&[5i64.into(), 1i64.into()]),
            Ok(Outcome::Returned(Some(15i64.into())))
        );
        assert_eq!(
            code.invoke(&[5i64.into(), (1i64 << 60).into()]),
            Ok(Outcome::Restart)
        );
    }

    /// The same loop without the late store keeps its resume: every local it
    /// can bind is established before the first guard is lowered, so no
    /// backward jump can carry in one the sites do not list.
    #[test]
    fn a_loop_whose_locals_are_all_established_still_resumes() {
        let code = jit_function! { mixed => r#"
def mixed(n: int, step: int) -> int:
    total = 0
    while n > 0:
        total = total + n * step
        n = n - 1
    return total
"# };
        match code.invoke(&[5i64.into(), (1i64 << 60).into()]) {
            Ok(Outcome::Deopt(state)) => {
                // Two iterations in: `total` holds 5 << 60 and the multiply's
                // 4 << 60 is on the stack under it, waiting for the addition
                // that overflowed.
                assert_eq!(state.locals[0], Some(AbiValue::Int(4)));
                assert_eq!(state.locals[1], Some(AbiValue::Int(1 << 60)));
                assert_eq!(state.locals[2], Some(AbiValue::Int(5 << 60)));
                assert_eq!(state.stack, vec![int(5 << 60), int(4 << 60)]);
            }
            other => panic!("expected a deopt, got {other:?}"),
        }
    }
}
