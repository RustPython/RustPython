#![cfg(test)]
#![cfg(all(feature = "compiler", feature = "gc"))]

//! `CodeType.replace(co_code=...)` accepts any byte sequence that decodes to
//! known opcodes, so an instruction operand can address a stack slot the frame
//! does not have. The interpreter has to reject that at run time instead of
//! reading outside the value stack (issue #10014).

use rustpython_vm::{Interpreter, bytecode::Opcode};

/// Replace the body of a two-instruction function with `opcode:oparg`.
///
/// The fixture keeps the compiler-generated frame layout (a zero-argument
/// code object with one stack slot) and only rewrites one instruction, which
/// is what `code.replace(co_code=...)` allows.
fn fixture(opcode: Opcode, oparg: u8) -> String {
    format!(
        r#"
def f():
    return None

raw = bytearray(f.__code__.co_code)
# RESUME; <load none>; RETURN_VALUE -> RESUME; <opcode>; RETURN_VALUE.
raw[-4] = {opcode}
raw[-3] = {oparg}
code = f.__code__.replace(co_code=bytes(raw))
exec(code)
"#,
        opcode = opcode.as_u8(),
        oparg = oparg,
    )
}

/// Run the fixture and assert that the interpreter rejects the stack index.
///
/// Reaching a bad index used to read outside the value stack in release
/// builds, so the checks under test are what keeps this from being undefined
/// behaviour rather than only a missing `debug_assert!`.
fn assert_rejected(vm: &Interpreter, source: &str, expected: &str) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = vm.enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            let code = vm
                .compile(
                    source,
                    rustpython_vm::compiler::Mode::Exec,
                    "<stack-bounds>",
                )
                .map_err(|error| error.into_pyexception(vm, Some(source)))
                .expect("the fixture itself must compile");
            vm.run_code_obj(code, scope).is_ok()
        });
    }));
    match result {
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().map(|s| &**s))
                .unwrap_or_default();
            assert!(
                message.contains(expected),
                "expected a {expected:?} rejection, got: {message}"
            );
        }
        Ok(()) => panic!("the interpreter ran an out-of-bounds stack index"),
    }
}

#[test]
fn copy_operand_past_stack_is_rejected() {
    let interpreter = Interpreter::without_stdlib(Default::default());
    assert_rejected(&interpreter, &fixture(Opcode::Copy, 100), "COPY index");
}

#[test]
fn swap_operand_past_stack_is_rejected() {
    let interpreter = Interpreter::without_stdlib(Default::default());
    assert_rejected(&interpreter, &fixture(Opcode::Swap, 200), "SWAP index");
}
