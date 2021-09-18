use super::pytype::PyTypeRef;
use crate::function::Args;
use crate::iterator;
use crate::slots::{PyIter, SlotConstructor};
use crate::vm::VirtualMachine;
use crate::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};

/// map(func, *iterables) --> map object
///
/// Make an iterator that computes the function using arguments from
/// each of the iterables. Stops when the shortest iterable is exhausted.
#[pyclass(module = false, name = "map")]
#[derive(Debug)]
pub struct PyMap {
    mapper: PyObjectRef,
    iterators: Vec<PyObjectRef>,
}

impl PyValue for PyMap {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.map_type
    }
}

impl SlotConstructor for PyMap {
    type Args = (PyObjectRef, Args<PyObjectRef>);

    fn py_new(cls: PyTypeRef, (function, iterables): Self::Args, vm: &VirtualMachine) -> PyResult {
        let iterators = iterables
            .into_iter()
            .map(|iterable| iterator::get_iter(vm, iterable))
            .collect::<Result<Vec<_>, _>>()?;
        PyMap {
            mapper: function,
            iterators,
        }
        .into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(PyIter, SlotConstructor), flags(BASETYPE))]
impl PyMap {
    #[pymethod(magic)]
    fn length_hint(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.iterators.iter().try_fold(0, |prev, cur| {
            let cur = iterator::length_hint(vm, cur.clone())?.unwrap_or(0);
            let max = std::cmp::max(prev, cur);
            Ok(max)
        })
    }
}

impl PyIter for PyMap {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let next_objs = zelf
            .iterators
            .iter()
            .map(|iterator| iterator::call_next(vm, iterator))
            .collect::<Result<Vec<_>, _>>()?;

        // the mapper itself can raise StopIteration which does stop the map iteration
        vm.invoke(&zelf.mapper, next_objs)
    }
}

pub fn init(context: &PyContext) {
    PyMap::extend_class(context, &context.types.map_type);
}
