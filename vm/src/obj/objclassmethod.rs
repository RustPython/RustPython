use crate::function::PyFuncArgs;
use crate::pyobject::{AttributeProtocol, PyContext, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    let classmethod_type = &context.classmethod_type;
    extend_class!(context, classmethod_type, {
        "__get__" => context.new_rustfunc(classmethod_get),
        "__new__" => context.new_rustfunc(classmethod_new)
    });
}

fn classmethod_get(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("classmethod.__get__ {:?}", args.args);
    arg_check!(
        vm,
        args,
        required = [
            (cls, Some(vm.ctx.classmethod_type())),
            (_inst, None),
            (owner, None)
        ]
    );
    match cls.get_attr("function") {
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
