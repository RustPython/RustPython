use super::super::pyobject::{
    IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objiter;
use super::objtype; // Required for arg_check! to use isinstance

pub fn filter_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None), (function, None), (iterable, None)]
    );
    let iterator = objiter::get_iter(vm, iterable)?;
    Ok(PyObject::new(
        PyObjectPayload::FilterIterator {
            predicate: function.clone(),
            iterator,
        },
        cls.clone(),
    ))
}

fn filter_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(filter, Some(vm.ctx.filter_type()))]);
    // Return self:
    Ok(filter.clone())
}

fn filter_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(filter, Some(vm.ctx.filter_type())), (needle, None)]
    );
    objiter::contains(vm, filter, needle)
}

fn filter_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(filter, Some(vm.ctx.filter_type()))]);

    if let PyObjectPayload::FilterIterator {
        ref mut predicate,
        ref mut iterator,
    } = filter.borrow_mut().payload
    {
        loop {
            let next_obj = objiter::call_next(vm, iterator)?;
            let predicate_value = if predicate.is(&vm.get_none()) {
                next_obj.clone()
            } else {
                // the predicate itself can raise StopIteration which does stop the filter
                // iteration
                vm.invoke(
                    predicate.clone(),
                    PyFuncArgs {
                        args: vec![next_obj.clone()],
                        kwargs: vec![],
                    },
                )?
            };
            if objbool::boolval(vm, predicate_value)? {
                return Ok(next_obj);
            }
        }
    } else {
        panic!("filter doesn't have correct payload");
    }
}

pub fn init(context: &PyContext) {
    let filter_type = &context.filter_type;
    context.set_attr(
        &filter_type,
        "__contains__",
        context.new_rustfunc(filter_contains),
    );
    context.set_attr(&filter_type, "__iter__", context.new_rustfunc(filter_iter));
    context.set_attr(&filter_type, "__new__", context.new_rustfunc(filter_new));
    context.set_attr(&filter_type, "__next__", context.new_rustfunc(filter_next));
}
