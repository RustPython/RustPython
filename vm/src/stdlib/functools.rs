use crate::function::OptionalArg;
use crate::obj::objiter;
use crate::obj::objtype;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "_functools", {
        "reduce" => ctx.new_rustfunc(functools_reduce),
    })
}

fn functools_reduce(
    function: PyObjectRef,
    sequence: PyObjectRef,
    start_value: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let iterator = objiter::get_iter(vm, &sequence)?;

    let start_value = if let OptionalArg::Present(val) = start_value {
        val
    } else {
        objiter::call_next(vm, &iterator).map_err(|err| {
            if objtype::isinstance(&err, &vm.ctx.exceptions.stop_iteration) {
                let exc_type = vm.ctx.exceptions.type_error.clone();
                vm.new_exception(
                    exc_type,
                    "reduce() of empty sequence with no initial value".to_string(),
                )
            } else {
                err
            }
        })?
    };

    let mut accumulator = start_value;

    while let Ok(next_obj) = objiter::call_next(vm, &iterator) {
        accumulator = vm.invoke(&function, vec![accumulator, next_obj])?
    }

    Ok(accumulator)
}
