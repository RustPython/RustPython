use super::{PyStr, PyType, PyTypeRef};
use crate::{
    builtins::builtinfunc::PyBuiltinMethod,
    class::PyClassImpl,
    function::{FuncArgs, IntoPyNativeFunc},
    types::{Callable, Constructor, GetDescriptor},
    Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};

#[pyclass(module = false, name = "staticmethod")]
#[derive(Clone, Debug)]
pub struct PyStaticMethod {
    pub callable: PyObjectRef,
}

impl PyPayload for PyStaticMethod {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.staticmethod_type
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
    pub fn new_builtin_ref<F, FKind>(
        name: impl Into<PyStr>,
        class: &'static Py<PyType>,
        f: F,
        ctx: &Context,
    ) -> PyRef<Self>
    where
        F: IntoPyNativeFunc<FKind>,
    {
        let callable = PyBuiltinMethod::new_ref(name, class, f, ctx).into();
        PyRef::new_ref(
            Self { callable },
            ctx.types.staticmethod_type.to_owned(),
            None,
        )
    }
}

#[pyimpl(with(Callable, GetDescriptor, Constructor), flags(BASETYPE, HAS_DICT))]
impl PyStaticMethod {
    #[pyproperty(magic)]
    fn func(&self) -> PyObjectRef {
        self.callable.clone()
    }

    #[pyproperty(magic)]
    fn wrapped(&self) -> PyObjectRef {
        self.callable.clone()
    }

    #[pyproperty(magic)]
    fn module(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.get_attr("__module__", vm)
    }

    #[pyproperty(magic)]
    fn qualname(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.get_attr("__qualname__", vm)
    }

    #[pyproperty(magic)]
    fn name(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.get_attr("__name__", vm)
    }

    #[pyproperty(magic)]
    fn annotations(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.get_attr("__annotations__", vm)
    }

    #[pyproperty(magic)]
    fn doc(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.get_attr("__doc__", vm)
    }

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
