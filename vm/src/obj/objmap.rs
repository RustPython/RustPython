use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objiter;
use super::objtype; // Required for arg_check! to use isinstance

pub fn map_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    no_kwargs!(vm, args);
    let cls = &args.args[0];
    if args.args.len() < 3 {
        Err(vm.new_type_error("map() must have at least two arguments.".to_owned()))
    } else {
        let function = &args.args[1];
        let iterables = &args.args[2..];
        let iterators = iterables
            .into_iter()
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

fn map_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(map, Some(vm.ctx.map_type()))]);
    // Return self:
    Ok(map.clone())
}

fn map_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(map, Some(vm.ctx.map_type())), (needle, None)]
    );
    objiter::contains(vm, map, needle)
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
    context.set_attr(
        &map_type,
        "__contains__",
        context.new_rustfunc(map_contains),
    );
    context.set_attr(&map_type, "__iter__", context.new_rustfunc(map_iter));
    context.set_attr(&map_type, "__new__", context.new_rustfunc(map_new));
    context.set_attr(&map_type, "__next__", context.new_rustfunc(map_next));
}
