use crate::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objiter;

#[derive(Debug)]
pub struct PyZip {
    iterators: Vec<PyObjectRef>,
}

impl PyValue for PyZip {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.zip_type()
    }
}

fn zip_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    no_kwargs!(vm, args);
    let cls = &args.args[0];
    let iterables = &args.args[1..];
    let iterators = iterables
        .iter()
        .map(|iterable| objiter::get_iter(vm, iterable))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PyObject::new(PyZip { iterators }, cls.clone()))
}

fn zip_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zip, Some(vm.ctx.zip_type()))]);

    if let Some(PyZip { ref iterators }) = zip.payload() {
        if iterators.is_empty() {
            Err(objiter::new_stop_iteration(vm))
        } else {
            let next_objs = iterators
                .iter()
                .map(|iterator| objiter::call_next(vm, iterator))
                .collect::<Result<Vec<_>, _>>()?;

            Ok(vm.ctx.new_tuple(next_objs))
        }
    } else {
        panic!("zip doesn't have correct payload");
    }
}

pub fn init(context: &PyContext) {
    let zip_type = &context.zip_type;
    objiter::iter_type_init(context, zip_type);
    extend_class!(context, zip_type, {
        "__new__" => context.new_rustfunc(zip_new),
        "__next__" => context.new_rustfunc(zip_next)
    });
}
