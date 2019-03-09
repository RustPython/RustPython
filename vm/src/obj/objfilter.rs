use crate::pyobject::{
    IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectPayload2, PyObjectRef,
    PyResult, TypeProtocol,
};
use crate::vm::VirtualMachine; // Required for arg_check! to use isinstance

use super::objbool;
use super::objiter;

#[derive(Debug)]
pub struct PyFilter {
    predicate: PyObjectRef,
    iterator: PyObjectRef,
}

impl PyObjectPayload2 for PyFilter {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.filter_type()
    }
}

fn filter_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None), (function, None), (iterable, None)]
    );
    let iterator = objiter::get_iter(vm, iterable)?;
    Ok(PyObject::new(
        PyObjectPayload::AnyRustValue {
            value: Box::new(PyFilter {
                predicate: function.clone(),
                iterator,
            }),
        },
        cls.clone(),
    ))
}

fn filter_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(filter, Some(vm.ctx.filter_type()))]);

    if let Some(PyFilter {
        ref predicate,
        ref iterator,
    }) = filter.payload()
    {
        loop {
            let next_obj = objiter::call_next(vm, iterator)?;
            let predicate_value = if predicate.is(&vm.get_none()) {
                next_obj.clone()
            } else {
                // the predicate itself can raise StopIteration which does stop the filter
                // iteration
                vm.invoke(predicate.clone(), vec![next_obj.clone()])?
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

    objiter::iter_type_init(context, filter_type);

    let filter_doc =
        "filter(function or None, iterable) --> filter object\n\n\
         Return an iterator yielding those items of iterable for which function(item)\n\
         is true. If function is None, return the items that are true.";

    context.set_attr(&filter_type, "__new__", context.new_rustfunc(filter_new));
    context.set_attr(
        &filter_type,
        "__doc__",
        context.new_str(filter_doc.to_string()),
    );
    context.set_attr(&filter_type, "__next__", context.new_rustfunc(filter_next));
}
