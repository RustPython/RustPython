use super::{PyStr, PyTypeRef};
use crate::{
    builtins::builtinfunc::PyBuiltinMethod,
    function::IntoPyNativeFunc,
    slots::{SlotConstructor, SlotDescriptor},
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, VirtualMachine,
};

#[pyclass(module = false, name = "staticmethod")]
#[derive(Clone, Debug)]
pub struct PyStaticMethod {
    pub callable: PyObjectRef,
}

impl PyValue for PyStaticMethod {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.staticmethod_type
    }
}

impl SlotDescriptor for PyStaticMethod {
    fn descr_get(
        zelf: PyObjectRef,
        _obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let zelf = Self::_zelf(zelf, vm)?;
        Ok(zelf.callable.clone())
    }
}

impl From<PyObjectRef> for PyStaticMethod {
    fn from(callable: PyObjectRef) -> Self {
        Self { callable }
    }
}

impl SlotConstructor for PyStaticMethod {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, callable: Self::Args, vm: &VirtualMachine) -> PyResult {
        PyStaticMethod { callable }.into_pyresult_with_type(vm, cls)
    }
}

impl PyStaticMethod {
    pub fn new_ref<F, FKind>(
        name: impl Into<PyStr>,
        class: PyTypeRef,
        f: F,
        ctx: &PyContext,
    ) -> PyRef<Self>
    where
        F: IntoPyNativeFunc<FKind>,
    {
        let callable = PyBuiltinMethod::new_ref(name, class, f, ctx).into();
        PyRef::new_ref(Self { callable }, ctx.types.staticmethod_type.clone(), None)
    }
}

#[pyimpl(with(SlotDescriptor, SlotConstructor), flags(BASETYPE, HAS_DICT))]
impl PyStaticMethod {}

pub fn init(context: &PyContext) {
    PyStaticMethod::extend_class(context, &context.types.staticmethod_type);
}
