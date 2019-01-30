/*! Python `property` descriptor class.

*/

use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let ref property_type = context.property_type;
    context.set_attr(
        &property_type,
        "__get__",
        context.new_rustfunc(property_get),
    );
    context.set_attr(
        &property_type,
        "__new__",
        context.new_rustfunc(property_new),
    );
    // TODO: how to handle __set__ ?
}

// `property` methods.
fn property_get(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("property.__get__ {:?}", args.args);
    arg_check!(
        vm,
        args,
        required = [
            (cls, Some(vm.ctx.property_type())),
            (inst, None),
            (_owner, None)
        ]
    );

    match vm.ctx.get_attr(&cls, "fget") {
        Some(getter) => {
            let py_method = vm.ctx.new_bound_method(getter, inst.clone());
            vm.invoke(py_method, PyFuncArgs::default())
        }
        None => {
            let attribute_error = vm.context().exceptions.attribute_error.clone();
            Err(vm.new_exception(
                attribute_error,
                String::from("Attribute Error: property must have 'fget' attribute"),
            ))
        }
    }
}

fn property_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("property.__new__ {:?}", args.args);
    arg_check!(vm, args, required = [(cls, None), (fget, None)]);

    let py_obj = PyObject::new(
        PyObjectPayload::Instance {
            dict: vm.ctx.new_dict(),
        },
        cls.clone(),
    );
    vm.ctx.set_attr(&py_obj, "fget", fget.clone());
    Ok(py_obj)
}
