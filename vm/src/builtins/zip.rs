use super::pytype::PyTypeRef;
use crate::function::Args;
use crate::iterator;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::slots::PyIter;
use crate::vm::VirtualMachine;

pub type PyZipRef = PyRef<PyZip>;

#[pyclass(module = false, name = "zip")]
#[derive(Debug)]
pub struct PyZip {
    iterators: Vec<PyObjectRef>,
}

impl PyValue for PyZip {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.zip_type
    }
}

#[pyimpl(with(PyIter), flags(BASETYPE))]
impl PyZip {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, iterables: Args, vm: &VirtualMachine) -> PyResult<PyZipRef> {
        let iterators = iterables
            .into_iter()
            .map(|iterable| iterator::get_iter(vm, iterable))
            .collect::<Result<Vec<_>, _>>()?;
        PyZip { iterators }.into_ref_with_type(vm, cls)
    }
}

impl PyIter for PyZip {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        if zelf.iterators.is_empty() {
            Err(vm.new_stop_iteration())
        } else {
            let next_objs = zelf
                .iterators
                .iter()
                .map(|iterator| iterator::call_next(vm, iterator))
                .collect::<Result<Vec<_>, _>>()?;

            Ok(vm.ctx.new_tuple(next_objs))
        }
    }
}

pub fn init(context: &PyContext) {
    PyZip::extend_class(context, &context.types.zip_type);
}
