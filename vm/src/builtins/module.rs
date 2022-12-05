use super::pystr::IntoPyStrRef;
use super::{PyDictRef, PyStr, PyStrRef, PyType, PyTypeRef};
use crate::{
    builtins::PyStrInterned,
    class::PyClassImpl,
    convert::ToPyObject,
    function::FuncArgs,
    types::{GetAttr, Initializer},
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};

#[pyclass(module = false, name = "module")]
#[derive(Debug)]
pub struct PyModule {}

impl PyPayload for PyModule {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.module_type
    }
}

#[derive(FromArgs)]
pub struct ModuleInitArgs {
    name: PyStrRef,
    #[pyarg(any, default)]
    doc: Option<PyStrRef>,
}

#[pyclass(with(GetAttr, Initializer), flags(BASETYPE, HAS_DICT))]
impl PyModule {
    // pub(crate) fn new(d: PyDictRef) -> Self {
    //     PyModule { dict: d.into() }
    // }

    // #[inline]
    // pub fn dict(&self) -> PyDictRef {
    //     self.dict.get()
    // }

    #[pyslot]
    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyModule {}.into_ref_with_type(vm, cls).map(Into::into)
    }

    fn getattr_inner(zelf: &Py<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        if let Some(attr) = zelf
            .as_object()
            .generic_getattr_opt(name.clone(), None, vm)?
        {
            return Ok(attr);
        }
        if let Ok(getattr) = zelf.dict().get_item(identifier!(vm, __getattr__), vm) {
            return vm.invoke(&getattr, (name,));
        }
        let module_name = if let Some(name) = Self::name(zelf.to_owned(), vm) {
            format!(" '{name}'")
        } else {
            "".to_owned()
        };
        Err(vm.new_attribute_error(format!("module{module_name} has no attribute '{name}'")))
    }

    fn name(zelf: PyRef<Self>, vm: &VirtualMachine) -> Option<PyStrRef> {
        zelf.as_object()
            .generic_getattr_opt(identifier!(vm, __name__).to_owned(), None, vm)
            .unwrap_or(None)
            .and_then(|obj| obj.downcast::<PyStr>().ok())
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let importlib = vm.import("_frozen_importlib", None, 0)?;
        let module_repr = importlib.get_attr("_module_repr", vm)?;
        vm.invoke(&module_repr, (zelf,))
    }

    #[pymethod(magic)]
    fn dir(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let dict = zelf
            .as_object()
            .dict()
            .ok_or_else(|| vm.new_value_error("module has no dict".to_owned()))?;
        let attrs = dict.into_iter().map(|(k, _v)| k).collect();
        Ok(attrs)
    }
}

impl Py<PyModule> {
    // TODO: to be replaced by the commented-out dict method above once dictoffsets land
    pub fn dict(&self) -> PyDictRef {
        self.as_object().dict().unwrap()
    }
    // TODO: should be on PyModule, not Py<PyModule>
    pub(crate) fn init_module_dict(
        &self,
        name: &'static PyStrInterned,
        doc: PyObjectRef,
        vm: &VirtualMachine,
    ) {
        let dict = self.dict();
        dict.set_item(identifier!(vm, __name__), name.to_object(), vm)
            .expect("Failed to set __name__ on module");
        dict.set_item(identifier!(vm, __doc__), doc, vm)
            .expect("Failed to set __doc__ on module");
        dict.set_item("__package__", vm.ctx.none(), vm)
            .expect("Failed to set __package__ on module");
        dict.set_item("__loader__", vm.ctx.none(), vm)
            .expect("Failed to set __loader__ on module");
        dict.set_item("__spec__", vm.ctx.none(), vm)
            .expect("Failed to set __spec__ on module");
    }

    pub fn get_attr(&self, attr_name: impl IntoPyStrRef, vm: &VirtualMachine) -> PyResult {
        PyModule::getattr_inner(self, attr_name.into_pystr_ref(vm), vm)
    }
    pub fn set_attr(
        &self,
        attr_name: impl IntoPyStrRef,
        attr_value: impl Into<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.as_object().set_attr(attr_name, attr_value, vm)
    }
}

impl Initializer for PyModule {
    type Args = ModuleInitArgs;

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        debug_assert!(zelf
            .class()
            .slots
            .flags
            .has_feature(crate::types::PyTypeFlags::HAS_DICT));
        zelf.init_module_dict(
            vm.ctx.intern_str(args.name.as_str()),
            args.doc.to_pyobject(vm),
            vm,
        );
        Ok(())
    }
}

impl GetAttr for PyModule {
    fn getattro(zelf: &Py<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        Self::getattr_inner(zelf, name, vm)
    }
}

pub(crate) fn init(context: &Context) {
    PyModule::extend_class(context, context.types.module_type);
}
