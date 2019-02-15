use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objiter;
use super::objtype; // Required for arg_check! to use isinstance

fn map_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    no_kwargs!(vm, args);
    let cls = &args.args[0];
    if args.args.len() < 3 {
        Err(vm.new_type_error("map() must have at least two arguments.".to_owned()))
    } else {
        let function = &args.args[1];
        let iterables = &args.args[2..];
        let iterators = iterables
            .iter()
            .map(|iterable| objiter::get_iter(vm, iterable))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PyObject::new(
            PyObjectPayload::MapIterator {
                mapper: function.clone(),
                iterators,
            },
            cls.clone(),
        ))
    }
}

fn map_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(map, Some(vm.ctx.map_type()))]);

    if let PyObjectPayload::MapIterator {
        ref mut mapper,
        ref mut iterators,
    } = map.borrow_mut().payload
    {
        let next_objs = iterators
            .iter()
            .map(|iterator| objiter::call_next(vm, iterator))
            .collect::<Result<Vec<_>, _>>()?;

        // the mapper itself can raise StopIteration which does stop the map iteration
        vm.invoke(
            mapper.clone(),
            PyFuncArgs {
                args: next_objs,
                kwargs: vec![],
            },
        )
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
