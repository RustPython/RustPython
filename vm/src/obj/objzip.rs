use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objiter;
use super::objtype; // Required for arg_check! to use isinstance

fn zip_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    no_kwargs!(vm, args);
    let cls = &args.args[0];
    let iterables = &args.args[1..];
    let iterators = iterables
        .iter()
        .map(|iterable| objiter::get_iter(vm, iterable))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PyObject::new(
        PyObjectPayload::ZipIterator { iterators },
        cls.clone(),
    ))
}

fn zip_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zip, Some(vm.ctx.zip_type()))]);

    if let PyObjectPayload::ZipIterator { ref mut iterators } = zip.borrow_mut().payload {
        if iterators.is_empty() {
            Err(objiter::new_stop_iteration(vm))
        } else {
            let next_objs = iterators
                .iter()
                .map(|iterator| objiter::call_next(vm, iterator))
                .collect::<Result<Vec<_>, _>>()?;

            Ok(vm.ctx.new_tuple(next_objs))
        }
    } else {
        panic!("zip doesn't have correct payload");
    }
}

pub fn init(context: &PyContext) {
    let zip_type = &context.zip_type;
    objiter::iter_type_init(context, zip_type);
    context.set_attr(zip_type, "__new__", context.new_rustfunc(zip_new));
    context.set_attr(zip_type, "__next__", context.new_rustfunc(zip_next));
}
