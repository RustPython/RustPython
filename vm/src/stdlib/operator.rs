use crate::byteslike::PyBytesLike;
use crate::function::OptionalArg;
use crate::obj::objstr::PyStringRef;
use crate::obj::{objiter, objtype};
use crate::pyobject::{BorrowValue, Either, PyObjectRef, PyResult, TypeProtocol};
use crate::VirtualMachine;
use volatile::Volatile;

fn operator_length_hint(obj: PyObjectRef, default: OptionalArg, vm: &VirtualMachine) -> PyResult {
    let default = default.unwrap_or_else(|| vm.ctx.new_int(0));
    if !objtype::isinstance(&default, &vm.ctx.types.int_type) {
        return Err(vm.new_type_error(format!(
            "'{}' type cannot be interpreted as an integer",
            default.class().name
        )));
    }
    let hint = objiter::length_hint(vm, obj)?
        .map(|i| vm.ctx.new_int(i))
        .unwrap_or(default);
    Ok(hint)
}

#[inline(never)]
#[cold]
fn timing_safe_cmp(a: &[u8], b: &[u8]) -> bool {
    // we use raw pointers here to keep faithful to the C implementation and
    // to try to avoid any optimizations rustc might do with slices
    let len_a = a.len();
    let a = a.as_ptr();
    let len_b = b.len();
    let b = b.as_ptr();
    /* The volatile type declarations make sure that the compiler has no
     * chance to optimize and fold the code in any way that may change
     * the timing.
     */
    let length: Volatile<usize>;
    let mut left: Volatile<*const u8>;
    let mut right: Volatile<*const u8>;
    let mut result: u8 = 0;

    /* loop count depends on length of b */
    length = Volatile::new(len_b);
    left = Volatile::new(std::ptr::null());
    right = Volatile::new(b);

    /* don't use else here to keep the amount of CPU instructions constant,
     * volatile forces re-evaluation
     *  */
    if len_a == length.read() {
        left.write(Volatile::new(a).read());
        result = 0;
    }
    if len_a != length.read() {
        left.write(b);
        result = 1;
    }

    for _ in 0..length.read() {
        let l = left.read();
        left.write(l.wrapping_add(1));
        let r = right.read();
        right.write(r.wrapping_add(1));
        // safety: the 0..length range will always be either:
        // * as long as the length of both a and b, if len_a and len_b are equal
        // * as long as b, and both `left` and `right` are b
        result |= unsafe { l.read_volatile() ^ r.read_volatile() };
    }

    result == 0
}

fn operator_compare_digest(
    a: Either<PyStringRef, PyBytesLike>,
    b: Either<PyStringRef, PyBytesLike>,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    let res = match (a, b) {
        (Either::A(a), Either::A(b)) => {
            if !a.borrow_value().is_ascii() || !b.borrow_value().is_ascii() {
                return Err(vm.new_type_error(
                    "comparing strings with non-ASCII characters is not supported".to_owned(),
                ));
            }
            timing_safe_cmp(a.borrow_value().as_bytes(), b.borrow_value().as_bytes())
        }
        (Either::B(a), Either::B(b)) => a.with_ref(|a| b.with_ref(|b| timing_safe_cmp(a, b))),
        _ => {
            return Err(vm
                .new_type_error("unsupported operand types(s) or combination of types".to_owned()))
        }
    };
    Ok(res)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "_operator", {
        "length_hint" => vm.ctx.new_function(operator_length_hint),
        "_compare_digest" => vm.ctx.new_function(operator_compare_digest),
    })
}
