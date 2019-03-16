use crate::function::PyFuncArgs;
use crate::pyobject::{AttributeProtocol, PyContext, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    let staticmethod_type = &context.staticmethod_type;
    extend_class!(context, staticmethod_type, {
        "__get__" => context.new_rustfunc(staticmethod_get),
        "__new__" => context.new_rustfunc(staticmethod_new),
    });
}

// `staticmethod` methods.
fn staticmethod_get(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("staticmethod.__get__ {:?}", args.args);
    arg_check!(
        vm,
        args,
        required = [
            (cls, Some(vm.ctx.staticmethod_type())),
            (_inst, None),
            (_owner, None)
        ]
    );
    match cls.get_attr("function") {
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
