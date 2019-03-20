use crate::function::{Args, PyFuncArgs};
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

use super::objiter;
use super::objtype::PyClassRef;

#[derive(Debug)]
pub struct PyMap {
    mapper: PyObjectRef,
    iterators: Vec<PyObjectRef>,
}
type PyMapRef = PyRef<PyMap>;

impl PyValue for PyMap {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.map_type()
    }
}

fn map_new(
    cls: PyClassRef,
    function: PyObjectRef,
    iterables: Args,
    vm: &mut VirtualMachine,
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

fn map_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(map, Some(vm.ctx.map_type()))]);

    if let Some(PyMap {
        ref mapper,
        ref iterators,
    }) = map.payload()
    {
        let next_objs = iterators
            .iter()
            .map(|iterator| objiter::call_next(vm, iterator))
            .collect::<Result<Vec<_>, _>>()?;

        // the mapper itself can raise StopIteration which does stop the map iteration
        vm.invoke(mapper.clone(), next_objs)
    } else {
        panic!("map doesn't have correct payload");
    }
}

pub fn init(context: &PyContext) {
    let map_type = &context.map_type;

    let map_doc = "map(func, *iterables) --> map object\n\n\
                   Make an iterator that computes the function using arguments from\n\
                   each of the iterables.  Stops when the shortest iterable is exhausted.";

    objiter::iter_type_init(context, map_type);
    context.set_attr(&map_type, "__new__", context.new_rustfunc(map_new));
    context.set_attr(&map_type, "__next__", context.new_rustfunc(map_next));
    context.set_attr(&map_type, "__doc__", context.new_str(map_doc.to_string()));
}
