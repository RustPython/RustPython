use super::dict::PyDictRef;
use super::pystr::{PyStr, PyStrRef};
use super::pytype::PyTypeRef;
use crate::function::FuncArgs;
use crate::pyobject::{
    BorrowValue, IntoPyObject, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult,
    PyValue,
};
use crate::slots::SlotGetattro;
use crate::vm::VirtualMachine;

#[pyclass(module = false, name = "module")]
#[derive(Debug)]
pub struct PyModule {}
pub type PyModuleRef = PyRef<PyModule>;

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
    #[pyarg(any)]
    name: PyStrRef,
    #[pyarg(any, default)]
    doc: Option<PyStrRef>,
}

#[pyimpl(with(SlotGetattro), flags(BASETYPE, HAS_DICT))]
impl PyModule {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyModule {}.into_ref_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn init(zelf: PyRef<Self>, args: ModuleInitArgs, vm: &VirtualMachine) {
        debug_assert!(crate::pyobject::TypeProtocol::class(zelf.as_object())
            .slots
            .flags
            .has_feature(crate::slots::PyTpFlags::HAS_DICT));
        init_module_dict(
            vm,
            &zelf.as_object().dict().unwrap(),
            args.name.into_object(),
            args.doc.into_pyobject(vm),
        );
    }

    fn name(zelf: PyRef<Self>, vm: &VirtualMachine) -> Option<String> {
        vm.generic_getattribute_opt(
            zelf.as_object().clone(),
            PyStr::from("__name__").into_ref(vm),
            None,
        )
        .unwrap_or(None)
        .and_then(|obj| obj.payload::<PyStr>().map(|s| s.borrow_value().to_owned()))
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let importlib = vm.import("_frozen_importlib", None, 0)?;
        let module_repr = vm.get_attribute(importlib, "_module_repr")?;
        vm.invoke(&module_repr, (zelf,))
    }

    #[pymethod(magic)]
    fn dir(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let dict = zelf
            .as_object()
            .dict()
            .ok_or_else(|| vm.new_value_error("module has no dict".to_owned()))?;
        let attrs = dict.into_iter().map(|(k, _v)| k).collect();
        Ok(vm.ctx.new_list(attrs))
    }
}

impl SlotGetattro for PyModule {
    fn getattro(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        if let Some(attr) =
            vm.generic_getattribute_opt(zelf.as_object().clone(), name.clone(), None)?
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
    PyModule::extend_class(&context, &context.types.module_type);
}
