use super::super::pyobject::{
    AttributeProtocol, IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef,
    PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let ref function_type = context.function_type;
    function_type.set_attr("__get__", context.new_rustfunc(bind_method));

    let ref member_descriptor_type = context.member_descriptor_type;
    member_descriptor_type.set_attr("__get__", context.new_rustfunc(member_get));

    let ref classmethod_type = context.classmethod_type;
    classmethod_type.set_attr("__get__", context.new_rustfunc(classmethod_get));
    classmethod_type.set_attr("__new__", context.new_rustfunc(classmethod_new));

    let ref staticmethod_type = context.staticmethod_type;
    staticmethod_type.set_attr("__get__", context.new_rustfunc(staticmethod_get));
    staticmethod_type.set_attr("__new__", context.new_rustfunc(staticmethod_new));
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

    let py_obj = PyObject::new(
        PyObjectKind::Instance {
            dict: vm.ctx.new_dict(),
        },
        cls.clone(),
    );
    py_obj.set_attr("function", callable.clone());
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

    let py_obj = PyObject::new(
        PyObjectKind::Instance {
            dict: vm.ctx.new_dict(),
        },
        cls.clone(),
    );
    py_obj.set_attr("function", callable.clone());
    Ok(py_obj)
}
