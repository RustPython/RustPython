use crate::frame::Scope;
use crate::pyobject::{
    AttributeProtocol, IdProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult, PyValue,
    TypeProtocol,
};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyFunction {
    // TODO: these shouldn't be public
    pub code: PyObjectRef,
    pub scope: Scope,
    pub defaults: PyObjectRef,
}

impl PyFunction {
    pub fn new(code: PyObjectRef, scope: Scope, defaults: PyObjectRef) -> Self {
        PyFunction {
            code,
            scope,
            defaults,
        }
    }
}

impl PyValue for PyFunction {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.function_type()
    }
}

#[derive(Debug)]
pub struct PyMethod {
    // TODO: these shouldn't be public
    pub object: PyObjectRef,
    pub function: PyObjectRef,
}

impl PyMethod {
    pub fn new(object: PyObjectRef, function: PyObjectRef) -> Self {
        PyMethod { object, function }
    }
}

impl PyValue for PyMethod {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.bound_method_type()
    }
}

pub fn init(context: &PyContext) {
    let function_type = &context.function_type;
    context.set_attr(&function_type, "__get__", context.new_rustfunc(bind_method));

    context.set_attr(
        &function_type,
        "__code__",
        context.new_property(function_code),
    );

    let builtin_function_or_method_type = &context.builtin_function_or_method_type;
    context.set_attr(
        &builtin_function_or_method_type,
        "__get__",
        context.new_rustfunc(bind_method),
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
    match args.args[0].payload() {
        Some(PyFunction { ref code, .. }) => Ok(code.clone()),
        None => Err(vm.new_type_error("no code".to_string())),
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
