use crate::function::{Args, KwArgs};
use crate::obj::objcode::PyCodeRef;
use crate::obj::objdict::PyDictRef;
use crate::obj::objtuple::PyTupleRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{IdProtocol, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::scope::Scope;
use crate::vm::VirtualMachine;

pub type PyFunctionRef = PyRef<PyFunction>;

#[derive(Debug)]
pub struct PyFunction {
    // TODO: these shouldn't be public
    pub code: PyCodeRef,
    pub scope: Scope,
    pub defaults: Option<PyTupleRef>,
    pub kw_only_defaults: Option<PyDictRef>,
    pub new_locals: bool,
}

impl PyFunction {
    pub fn new(
        code: PyCodeRef,
        scope: Scope,
        defaults: Option<PyTupleRef>,
        kw_only_defaults: Option<PyDictRef>,
        new_locals: bool,
    ) -> Self {
        PyFunction {
            code,
            scope,
            defaults,
            kw_only_defaults,
            new_locals,
        }
    }
}

impl PyValue for PyFunction {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.function_type()
    }
}

impl PyFunctionRef {
    fn call(self, args: Args, kwargs: KwArgs, vm: &VirtualMachine) -> PyResult {
        vm.invoke(&self.into_object(), (&args, &kwargs))
    }

    fn code(self, _vm: &VirtualMachine) -> PyCodeRef {
        self.code.clone()
    }

    fn defaults(self, _vm: &VirtualMachine) -> Option<PyTupleRef> {
        self.defaults.clone()
    }

    fn kwdefaults(self, _vm: &VirtualMachine) -> Option<PyDictRef> {
        self.kw_only_defaults.clone()
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
    let function_type = &context.types.function_type;
    extend_class!(context, function_type, {
        "__get__" => context.new_rustfunc(bind_method),
        "__call__" => context.new_rustfunc(PyFunctionRef::call),
        "__code__" => context.new_property(PyFunctionRef::code),
        "__defaults__" => context.new_property(PyFunctionRef::defaults),
        "__kwdefaults__" => context.new_property(PyFunctionRef::kwdefaults),
    });

    let builtin_function_or_method_type = &context.types.builtin_function_or_method_type;
    extend_class!(context, builtin_function_or_method_type, {
        "__get__" => context.new_rustfunc(bind_method)
    });
}

fn bind_method(
    function: PyObjectRef,
    obj: PyObjectRef,
    cls: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult {
    if obj.is(&vm.get_none()) && !cls.is(&obj.class()) {
        Ok(function)
    } else {
        Ok(vm.ctx.new_bound_method(function, obj))
    }
}
