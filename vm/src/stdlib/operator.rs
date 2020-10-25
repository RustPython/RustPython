use crate::builtins::pystr::PyStrRef;
use crate::byteslike::PyBytesLike;
use crate::common::cmp;
use crate::function::OptionalArg;
use crate::iterator;
use crate::pyobject::{BorrowValue, Either, PyObjectRef, PyResult, TypeProtocol};
use crate::VirtualMachine;

fn _operator_length_hint(obj: PyObjectRef, default: OptionalArg, vm: &VirtualMachine) -> PyResult {
    let default = default.unwrap_or_else(|| vm.ctx.new_int(0));
    if !default.isinstance(&vm.ctx.types.int_type) {
        return Err(vm.new_type_error(format!(
            "'{}' type cannot be interpreted as an integer",
            default.class().name
        )));
    }
    let hint = iterator::length_hint(vm, obj)?
        .map(|i| vm.ctx.new_int(i))
        .unwrap_or(default);
    Ok(hint)
}

fn _operator_compare_digest(
    a: Either<PyStrRef, PyBytesLike>,
    b: Either<PyStrRef, PyBytesLike>,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    let res = match (a, b) {
        (Either::A(a), Either::A(b)) => {
            if !a.borrow_value().is_ascii() || !b.borrow_value().is_ascii() {
                return Err(vm.new_type_error(
                    "comparing strings with non-ASCII characters is not supported".to_owned(),
                ));
            }
            cmp::timing_safe_cmp(a.borrow_value().as_bytes(), b.borrow_value().as_bytes())
        }
        (Either::B(a), Either::B(b)) => a.with_ref(|a| b.with_ref(|b| cmp::timing_safe_cmp(a, b))),
        _ => {
            return Err(vm
                .new_type_error("unsupported operand types(s) or combination of types".to_owned()))
        }
    };
    Ok(res)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "_operator", {
        "length_hint" => named_function!(ctx, _operator, length_hint),
        "_compare_digest" => named_function!(ctx, _operator, compare_digest),
    })
}
