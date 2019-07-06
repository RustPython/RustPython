use crate::function::Args;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use super::objiter;
use super::objtype::PyClassRef;

/// map(func, *iterables) --> map object
///
/// Make an iterator that computes the function using arguments from
/// each of the iterables.  Stops when the shortest iterable is exhausted.
#[pyclass]
#[derive(Debug)]
pub struct PyMap {
    mapper: PyObjectRef,
    iterators: Vec<PyObjectRef>,
}
type PyMapRef = PyRef<PyMap>;

impl PyValue for PyMap {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.map_type()
    }
}

fn map_new(
    cls: PyClassRef,
    function: PyObjectRef,
    iterables: Args,
    vm: &VirtualMachine,
) -> PyResult<PyMapRef> {
    let iterators = iterables
        .into_iter()
        .map(|iterable| objiter::get_iter(vm, &iterable))
        .collect::<Result<Vec<_>, _>>()?;
    PyMap {
        mapper: function.clone(),
        iterators,
    }
    .into_ref_with_type(vm, cls.clone())
}

#[pyimpl]
impl PyMap {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let next_objs = self
            .iterators
            .iter()
            .map(|iterator| objiter::call_next(vm, iterator))
            .collect::<Result<Vec<_>, _>>()?;

        // the mapper itself can raise StopIteration which does stop the map iteration
        vm.invoke(self.mapper.clone(), next_objs)
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

pub fn init(context: &PyContext) {
    PyMap::extend_class(context, &context.map_type);
    extend_class!(context, &context.map_type, {
        "__new__" => context.new_rustfunc(map_new),
    });
}
