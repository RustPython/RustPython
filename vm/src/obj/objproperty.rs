/*! Python `property` descriptor class.

*/

use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let ref property_type = context.property_type;
    property_type.set_attr("__get__", context.new_rustfunc(property_get));
    property_type.set_attr("__new__", context.new_rustfunc(property_new));
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

    match cls.get_attr("fget") {
        Some(getter) => {
            let py_method = vm.new_bound_method(getter, inst.clone());
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
        PyObjectKind::Instance {
            dict: vm.ctx.new_dict(),
        },
        cls.clone(),
    );
    py_obj.set_attr("fget", fget.clone());
    Ok(py_obj)
}
