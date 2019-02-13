use super::super::pyobject::{
    AttributeProtocol, IdProtocol, PyContext, PyFuncArgs, PyObjectPayload, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let function_type = &context.function_type;
    context.set_attr(&function_type, "__get__", context.new_rustfunc(bind_method));

    context.set_attr(
        &function_type,
        "__code__",
        context.new_member_descriptor(function_code),
    );

    let builtin_function_or_method_type = &context.builtin_function_or_method_type;
    context.set_attr(
        &builtin_function_or_method_type,
        "__get__",
        context.new_rustfunc(bind_method),
    );

    let member_descriptor_type = &context.member_descriptor_type;
    context.set_attr(
        &member_descriptor_type,
        "__get__",
        context.new_rustfunc(member_get),
    );

    let classmethod_type = &context.classmethod_type;
    context.set_attr(
        &classmethod_type,
        "__get__",
        context.new_rustfunc(classmethod_get),
    );
    context.set_attr(
        &classmethod_type,
        "__new__",
        context.new_rustfunc(classmethod_new),
    );

    let staticmethod_type = &context.staticmethod_type;
    context.set_attr(
        staticmethod_type,
        "__get__",
        context.new_rustfunc(staticmethod_get),
    );
    context.set_attr(
        staticmethod_type,
        "__new__",
        context.new_rustfunc(staticmethod_new),
    );
}

fn bind_method(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(function, None), (obj, None), (cls, None)]
    );

    if obj.is(&vm.get_none()) && !cls.is(&obj.typ()) {
        Ok(function.clone())
    } else {
        Ok(vm.ctx.new_bound_method(function.clone(), obj.clone()))
    }
}

fn function_code(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    match args.args[0].borrow().payload {
        PyObjectPayload::Function { ref code, .. } => Ok(code.clone()),
        _ => Err(vm.new_type_error("no code".to_string())),
    }
}

fn member_get(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    match args.shift().get_attr("function") {
        Some(function) => vm.invoke(function, args),
        None => {
            let attribute_error = vm.context().exceptions.attribute_error.clone();
            Err(vm.new_exception(attribute_error, String::from("Attribute Error")))
        }
    }
}

// Classmethod type methods:
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
        None => {
            let attribute_error = vm.context().exceptions.attribute_error.clone();
            Err(vm.new_exception(
                attribute_error,
                String::from("Attribute Error: classmethod must have 'function' attribute"),
            ))
        }
    }
}

fn classmethod_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("classmethod.__new__ {:?}", args.args);
    arg_check!(vm, args, required = [(cls, None), (callable, None)]);

    let py_obj = vm.ctx.new_instance(cls.clone(), None);
    vm.ctx.set_attr(&py_obj, "function", callable.clone());
    Ok(py_obj)
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
        None => {
            let attribute_error = vm.context().exceptions.attribute_error.clone();
            Err(vm.new_exception(
                attribute_error,
                String::from("Attribute Error: staticmethod must have 'function' attribute"),
            ))
        }
    }
}

fn staticmethod_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("staticmethod.__new__ {:?}", args.args);
    arg_check!(vm, args, required = [(cls, None), (callable, None)]);

    let py_obj = vm.ctx.new_instance(cls.clone(), None);
    vm.ctx.set_attr(&py_obj, "function", callable.clone());
    Ok(py_obj)
}
