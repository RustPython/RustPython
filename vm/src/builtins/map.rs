use super::PyTypeRef;
use crate::{
    builtins::PyTupleRef,
    class::PyClassImpl,
    function::PosArgs,
    protocol::{PyIter, PyIterReturn},
    types::{Constructor, IterNext, IterNextIterable},
    Context, PyObjectRef, PyPayload, PyResult, VirtualMachine,
};

/// map(func, *iterables) --> map object
///
/// Make an iterator that computes the function using arguments from
/// each of the iterables. Stops when the shortest iterable is exhausted.
#[pyclass(module = false, name = "map")]
#[derive(Debug)]
pub struct PyMap {
    mapper: PyObjectRef,
    iterators: Vec<PyIter>,
}

impl PyPayload for PyMap {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.map_type
    }
}

impl Constructor for PyMap {
    type Args = (PyObjectRef, PosArgs<PyIter>);

    fn py_new(cls: PyTypeRef, (mapper, iterators): Self::Args, vm: &VirtualMachine) -> PyResult {
        let iterators = iterators.into_vec();
        PyMap { mapper, iterators }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

#[pyimpl(with(IterNext, Constructor), flags(BASETYPE))]
impl PyMap {
    #[pymethod(magic)]
    fn length_hint(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.iterators.iter().try_fold(0, |prev, cur| {
            let cur = cur.as_ref().to_owned().length_hint(0, vm)?;
            let max = std::cmp::max(prev, cur);
            Ok(max)
        })
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> (PyTypeRef, PyTupleRef) {
        let mut vec = vec![self.mapper.clone()];
        vec.extend(self.iterators.iter().map(|o| o.clone().into()));
        (vm.ctx.types.map_type.clone(), vm.new_tuple(vec))
    }
}

impl IterNextIterable for PyMap {}
impl IterNext for PyMap {
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let mut next_objs = Vec::new();
        for iterator in &zelf.iterators {
            let item = match iterator.next(vm)? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            };
            next_objs.push(item);
        }

        // the mapper itself can raise StopIteration which does stop the map iteration
        PyIterReturn::from_pyresult(vm.invoke(&zelf.mapper, next_objs), vm)
    }
}

pub fn init(context: &Context) {
    PyMap::extend_class(context, &context.types.map_type);
}
