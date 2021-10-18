use super::{PyDictRef, PyStr, PyStrRef, PyTypeRef};
use crate::{
    function::{FuncArgs, IntoPyObject},
    types::GetAttr,
    ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, VirtualMachine,
};

#[pyclass(module = false, name = "module")]
#[derive(Debug)]
pub struct PyModule;

impl PyValue for PyModule {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.module_type
    }
}

pub fn init_module_dict(
    vm: &VirtualMachine,
    module_dict: &PyDictRef,
    name: PyObjectRef,
    doc: PyObjectRef,
) {
    module_dict
        .set_item("__name__", name, vm)
        .expect("Failed to set __name__ on module");
    module_dict
        .set_item("__doc__", doc, vm)
        .expect("Failed to set __doc__ on module");
    module_dict
        .set_item("__package__", vm.ctx.none(), vm)
        .expect("Failed to set __package__ on module");
    module_dict
        .set_item("__loader__", vm.ctx.none(), vm)
        .expect("Failed to set __loader__ on module");
    module_dict
        .set_item("__spec__", vm.ctx.none(), vm)
        .expect("Failed to set __spec__ on module");
}

#[derive(FromArgs)]
struct ModuleInitArgs {
    name: PyStrRef,
    #[pyarg(any, default)]
    doc: Option<PyStrRef>,
}

#[pyimpl(with(GetAttr), flags(BASETYPE, HAS_DICT))]
impl PyModule {
    #[pyslot]
    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyModule {}.into_pyresult_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn init(zelf: PyRef<Self>, args: ModuleInitArgs, vm: &VirtualMachine) {
        debug_assert!(crate::TypeProtocol::class(zelf.as_object())
            .slots
            .flags
            .has_feature(crate::types::PyTypeFlags::HAS_DICT));
        init_module_dict(
            vm,
            &zelf.as_object().dict().unwrap(),
            args.name.into(),
            args.doc.into_pyobject(vm),
        );
    }

    fn name(zelf: PyRef<Self>, vm: &VirtualMachine) -> Option<String> {
        vm.generic_getattribute_opt(
            zelf.as_object().incref(),
            PyStr::from("__name__").into_ref(vm),
            None,
        )
        .unwrap_or(None)
        .and_then(|obj| obj.payload::<PyStr>().map(|s| s.as_str().to_owned()))
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

impl GetAttr for PyModule {
    fn getattro(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        if let Some(attr) =
            vm.generic_getattribute_opt(zelf.as_object().incref(), name.clone(), None)?
        {
            return Ok(attr);
        }
        if let Some(getattr) = zelf
            .as_object()
            .dict()
            .and_then(|d| d.get_item("__getattr__", vm).ok())
        {
            return vm.invoke(&getattr, (name,));
        }
        let module_name = if let Some(name) = Self::name(zelf, vm) {
            format!(" '{}'", name)
        } else {
            "".to_owned()
        };
        Err(vm.new_attribute_error(format!("module{} has no attribute '{}'", module_name, name)))
    }
}

pub(crate) fn init(context: &PyContext) {
    PyModule::extend_class(context, &context.types.module_type);
}
