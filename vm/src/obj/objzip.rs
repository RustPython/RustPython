use crate::function::Args;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
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

impl PyZipRef {
    fn next(self, vm: &VirtualMachine) -> PyResult {
        if self.iterators.is_empty() {
            Err(objiter::new_stop_iteration(vm))
        } else {
            let next_objs = self
                .iterators
                .iter()
                .map(|iterator| objiter::call_next(vm, iterator))
                .collect::<Result<Vec<_>, _>>()?;

            Ok(vm.ctx.new_tuple(next_objs))
        }
    }

    fn iter(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

pub fn init(context: &PyContext) {
    let zip_type = &context.zip_type;
    extend_class!(context, zip_type, {
        "__new__" => context.new_rustfunc(zip_new),
        "__next__" => context.new_rustfunc(PyZipRef::next),
        "__iter__" => context.new_rustfunc(PyZipRef::iter),
    });
}
