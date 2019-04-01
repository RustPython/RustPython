use crate::pyobject::{IdProtocol, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine; // Required for arg_check! to use isinstance

use super::objbool;
use super::objiter;
use crate::obj::objtype::PyClassRef;

pub type PyFilterRef = PyRef<PyFilter>;

#[derive(Debug)]
pub struct PyFilter {
    predicate: PyObjectRef,
    iterator: PyObjectRef,
}

impl PyValue for PyFilter {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.filter_type()
    }
}

fn filter_new(
    cls: PyClassRef,
    function: PyObjectRef,
    iterable: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyFilterRef> {
    let iterator = objiter::get_iter(vm, &iterable)?;

    PyFilter {
        predicate: function.clone(),
        iterator,
    }
    .into_ref_with_type(vm, cls)
}

impl PyFilterRef {
    fn next(self, vm: &VirtualMachine) -> PyResult {
        let predicate = &self.predicate;
        let iterator = &self.iterator;
        loop {
            let next_obj = objiter::call_next(vm, iterator)?;
            let predicate_value = if predicate.is(&vm.get_none()) {
                next_obj.clone()
            } else {
                // the predicate itself can raise StopIteration which does stop the filter
                // iteration
                vm.invoke(predicate.clone(), vec![next_obj.clone()])?
            };
            if objbool::boolval(vm, predicate_value)? {
                return Ok(next_obj);
            }
        }
    }

    fn iter(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

pub fn init(context: &PyContext) {
    let filter_type = &context.filter_type;

    let filter_doc =
        "filter(function or None, iterable) --> filter object\n\n\
         Return an iterator yielding those items of iterable for which function(item)\n\
         is true. If function is None, return the items that are true.";

    extend_class!(context, filter_type, {
        "__new__" => context.new_rustfunc(filter_new),
        "__doc__" => context.new_str(filter_doc.to_string()),
        "__next__" => context.new_rustfunc(PyFilterRef::next),
        "__iter__" => context.new_rustfunc(PyFilterRef::iter),
    });
}
