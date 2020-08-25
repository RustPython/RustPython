use super::objiter;
use super::objtype::PyClassRef;
use crate::function::Args;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

/// map(func, *iterables) --> map object
///
/// Make an iterator that computes the function using arguments from
/// each of the iterables.  Stops when the shortest iterable is exhausted.
#[pyclass(module = false, name = "map")]
#[derive(Debug)]
pub struct PyMap {
    mapper: PyObjectRef,
    iterators: Vec<PyObjectRef>,
}
type PyMapRef = PyRef<PyMap>;

impl PyValue for PyMap {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.map_type.clone()
    }
}

#[pyimpl(flags(BASETYPE))]
impl PyMap {
    #[pyslot]
    fn tp_new(
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
            mapper: function,
            iterators,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let next_objs = self
            .iterators
            .iter()
            .map(|iterator| objiter::call_next(vm, iterator))
            .collect::<Result<Vec<_>, _>>()?;

        // the mapper itself can raise StopIteration which does stop the map iteration
        vm.invoke(&self.mapper, next_objs)
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__length_hint__")]
    fn length_hint(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.iterators.iter().try_fold(0, |prev, cur| {
            let cur = objiter::length_hint(vm, cur.clone())?.unwrap_or(0);
            let max = std::cmp::max(prev, cur);
            Ok(max)
        })
    }
}

pub fn init(context: &PyContext) {
    PyMap::extend_class(context, &context.types.map_type);
}
