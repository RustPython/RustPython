use super::{PyType, PyTypeRef};
use crate::{
    class::PyClassImpl,
    function::FuncArgs,
    types::{Callable, Constructor, GetDescriptor},
    Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
};

#[pyclass(module = false, name = "staticmethod")]
#[derive(Clone, Debug)]
pub struct PyStaticMethod {
    pub callable: PyObjectRef,
}

impl PyPayload for PyStaticMethod {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.staticmethod_type
    }
}

impl GetDescriptor for PyStaticMethod {
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

impl Constructor for PyStaticMethod {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, callable: Self::Args, vm: &VirtualMachine) -> PyResult {
        PyStaticMethod { callable }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

impl PyStaticMethod {
    pub fn new(callable: PyObjectRef) -> Self {
        Self { callable }
    }
}

#[pyimpl(with(Callable, GetDescriptor, Constructor), flags(BASETYPE, HAS_DICT))]
impl PyStaticMethod {
    #[pyproperty(magic)]
    fn isabstractmethod(&self, vm: &VirtualMachine) -> PyObjectRef {
        match vm.get_attribute_opt(self.callable.clone(), "__isabstractmethod__") {
            Ok(Some(is_abstract)) => is_abstract,
            _ => vm.ctx.new_bool(false).into(),
        }
    }

    #[pyproperty(magic, setter)]
    fn set_isabstractmethod(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.callable.set_attr("__isabstractmethod__", value, vm)?;
        Ok(())
    }
}

impl Callable for PyStaticMethod {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &crate::Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        vm.invoke(&zelf.callable, args)
    }
}

pub fn init(context: &Context) {
    PyStaticMethod::extend_class(context, context.types.staticmethod_type);
}
