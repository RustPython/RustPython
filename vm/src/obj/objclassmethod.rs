use super::objtype::{class_get_attr, PyClassRef};
use crate::function::PyFuncArgs;
use crate::pyobject::{PyContext, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    let classmethod_type = &context.classmethod_type;
    extend_class!(context, classmethod_type, {
        "__get__" => context.new_rustfunc(classmethod_get),
        "__new__" => context.new_rustfunc(classmethod_new)
    });
}

fn classmethod_get(
    class: PyClassRef,
    _inst: PyObjectRef,
    owner: PyObjectRef,
    vm: &mut VirtualMachine,
) -> PyResult {
    trace!("classmethod.__get__ {:?}", class);
    match class_get_attr(&class, "function") {
        Some(function) => {
            let py_obj = owner.clone();
            let py_method = vm.ctx.new_bound_method(function, py_obj);
            Ok(py_method)
        }
        None => Err(vm.new_attribute_error(
            "Attribute Error: classmethod must have 'function' attribute".to_string(),
        )),
    }
}

fn classmethod_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("classmethod.__new__ {:?}", args.args);
    arg_check!(vm, args, required = [(cls, None), (callable, None)]);

    let py_obj = vm.ctx.new_instance(cls.clone(), None);
    vm.ctx.set_attr(&py_obj, "function", callable.clone());
    Ok(py_obj)
}
