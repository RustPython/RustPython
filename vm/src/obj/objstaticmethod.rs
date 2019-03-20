use super::objtype::{class_get_attr, PyClassRef};
use crate::function::PyFuncArgs;
use crate::pyobject::{PyContext, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    let staticmethod_type = &context.staticmethod_type;
    extend_class!(context, staticmethod_type, {
        "__get__" => context.new_rustfunc(staticmethod_get),
        "__new__" => context.new_rustfunc(staticmethod_new),
    });
}

// `staticmethod` methods.
fn staticmethod_get(
    class: PyClassRef,
    _inst: PyObjectRef,
    _owner: PyObjectRef,
    vm: &mut VirtualMachine,
) -> PyResult {
    trace!("staticmethod.__get__ {:?}", class);
    match class_get_attr(&class, "function") {
        Some(function) => Ok(function),
        None => Err(vm.new_attribute_error(
            "Attribute Error: staticmethod must have 'function' attribute".to_string(),
        )),
    }
}

fn staticmethod_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("staticmethod.__new__ {:?}", args.args);
    arg_check!(vm, args, required = [(cls, None), (callable, None)]);

    let py_obj = vm.ctx.new_instance(cls.clone(), None);
    vm.ctx.set_attr(&py_obj, "function", callable.clone());
    Ok(py_obj)
}
