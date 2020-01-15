use super::objcode::PyCodeRef;
use super::objdict::PyDictRef;
use super::objstr::PyStringRef;
use super::objtuple::PyTupleRef;
use super::objtype::PyClassRef;
use crate::descriptor::PyBuiltinDescriptor;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::scope::Scope;
use crate::vm::VirtualMachine;

pub type PyFunctionRef = PyRef<PyFunction>;

#[pyclass]
#[derive(Debug)]
pub struct PyFunction {
    // TODO: these shouldn't be public
    pub code: PyCodeRef,
    scope: Scope,
    pub defaults: Option<PyTupleRef>,
    pub kw_only_defaults: Option<PyDictRef>,
}

impl PyBuiltinDescriptor for PyFunction {
    fn get(
        zelf: PyRef<Self>,
        obj: PyObjectRef,
        cls: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if obj.is(&vm.get_none()) && !Self::_cls_is(&cls, &obj.class()) {
            Ok(zelf.into_object())
        } else {
            Ok(vm.ctx.new_bound_method(zelf.into_object(), obj))
        }
    }
}

impl PyFunction {
    pub fn new(
        code: PyCodeRef,
        scope: Scope,
        defaults: Option<PyTupleRef>,
        kw_only_defaults: Option<PyDictRef>,
    ) -> Self {
        PyFunction {
            code,
            scope,
            defaults,
            kw_only_defaults,
        }
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }
}

impl PyValue for PyFunction {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.function_type()
    }
}

#[pyimpl]
impl PyFunction {
    #[pymethod(name = "__call__")]
    fn call(zelf: PyObjectRef, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        vm.invoke(&zelf, args)
    }

    #[pyproperty(name = "__code__")]
    fn code(&self, _vm: &VirtualMachine) -> PyCodeRef {
        self.code.clone()
    }

    #[pyproperty(name = "__defaults__")]
    fn defaults(&self, _vm: &VirtualMachine) -> Option<PyTupleRef> {
        self.defaults.clone()
    }

    #[pyproperty(name = "__kwdefaults__")]
    fn kwdefaults(&self, _vm: &VirtualMachine) -> Option<PyDictRef> {
        self.kw_only_defaults.clone()
    }
}

#[derive(Debug)]
pub struct PyBoundMethod {
    // TODO: these shouldn't be public
    pub object: PyObjectRef,
    pub function: PyObjectRef,
}

impl PyBoundMethod {
    pub fn new(object: PyObjectRef, function: PyObjectRef) -> Self {
        PyBoundMethod { object, function }
    }

    fn getattribute(&self, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        vm.get_attribute(self.function.clone(), name)
    }
}

impl PyValue for PyBoundMethod {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bound_method_type()
    }
}

pub fn init(context: &PyContext) {
    let function_type = &context.types.function_type;
    PyFunction::extend_class(context, function_type);
    extend_class!(context, function_type, {
        "__get__" => context.new_method(PyFunction::get),
        (slot descr_get) => PyFunction::get,
    });

    let method_type = &context.types.bound_method_type;
    extend_class!(context, method_type, {
        "__getattribute__" => context.new_method(PyBoundMethod::getattribute),
    });
}
