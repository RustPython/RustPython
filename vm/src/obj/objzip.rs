use crate::function::{Args, PyFuncArgs};
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

use super::objiter;
use crate::obj::objtype::PyClassRef;

pub type PyZipRef = PyRef<PyZip>;

#[derive(Debug)]
pub struct PyZip {
    iterators: Vec<PyObjectRef>,
}

impl PyValue for PyZip {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.zip_type()
    }
}

fn zip_new(cls: PyClassRef, iterables: Args, vm: &VirtualMachine) -> PyResult<PyZipRef> {
    let iterators = iterables
        .into_iter()
        .map(|iterable| objiter::get_iter(vm, &iterable))
        .collect::<Result<Vec<_>, _>>()?;
    PyZip { iterators }.into_ref_with_type(vm, cls)
}

fn zip_next(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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
