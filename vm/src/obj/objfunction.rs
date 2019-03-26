use crate::frame::Scope;
use crate::function::PyFuncArgs;
use crate::obj::objcode::PyCodeRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{IdProtocol, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

pub type PyFunctionRef = PyRef<PyFunction>;

#[derive(Debug)]
pub struct PyFunction {
    // TODO: these shouldn't be public
    pub code: PyCodeRef,
    pub scope: Scope,
    pub defaults: PyObjectRef,
}

impl PyFunction {
    pub fn new(code: PyCodeRef, scope: Scope, defaults: PyObjectRef) -> Self {
        PyFunction {
            code,
            scope,
            defaults,
        }
    }
}

impl PyValue for PyFunction {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.function_type()
    }
}

impl PyFunctionRef {
    fn code(self, _vm: &VirtualMachine) -> PyCodeRef {
        self.code.clone()
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
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bound_method_type()
    }
}

pub fn init(context: &PyContext) {
    let function_type = &context.function_type;
    extend_class!(context, function_type, {
        "__get__" => context.new_rustfunc(bind_method),
        "__code__" => context.new_property(PyFunctionRef::code)
    });

    let builtin_function_or_method_type = &context.builtin_function_or_method_type;
    extend_class!(context, builtin_function_or_method_type, {
        "__get__" => context.new_rustfunc(bind_method)
    });
}

fn bind_method(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(function, None), (obj, None), (cls, None)]
    );

    if obj.is(&vm.get_none()) && !cls.is(&obj.class()) {
        Ok(function.clone())
    } else {
        Ok(vm.ctx.new_bound_method(function.clone(), obj.clone()))
    }
}
