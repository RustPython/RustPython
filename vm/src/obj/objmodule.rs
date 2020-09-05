use super::objdict::PyDictRef;
use super::objstr::{PyString, PyStringRef};
use super::objtype::PyClassRef;
use crate::function::{OptionalOption, PyFuncArgs};
use crate::pyobject::{
    BorrowValue, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::vm::VirtualMachine;

#[pyclass(module = false, name = "module")]
#[derive(Debug)]
pub struct PyModule {}
pub type PyModuleRef = PyRef<PyModule>;

impl PyValue for PyModule {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.module_type.clone()
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
        .set_item("__package__", vm.get_none(), vm)
        .expect("Failed to set __package__ on module");
    module_dict
        .set_item("__loader__", vm.get_none(), vm)
        .expect("Failed to set __loader__ on module");
    module_dict
        .set_item("__spec__", vm.get_none(), vm)
        .expect("Failed to set __spec__ on module");
}

#[pyimpl(flags(BASETYPE, HAS_DICT))]
impl PyModuleRef {
    #[pyslot]
    fn tp_new(cls: PyClassRef, _args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<PyModuleRef> {
        PyModule {}.into_ref_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn init(
        self,
        name: PyStringRef,
        doc: OptionalOption<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        debug_assert!(crate::pyobject::TypeProtocol::lease_class(self.as_object())
            .slots
            .flags
            .has_feature(crate::slots::PyTpFlags::HAS_DICT));
        init_module_dict(
            vm,
            &self.as_object().dict().unwrap(),
            name.into_object(),
            doc.flatten()
                .map_or_else(|| vm.get_none(), PyRef::into_object),
        );
        Ok(())
    }

    fn name(self, vm: &VirtualMachine) -> Option<String> {
        vm.generic_getattribute_opt(
            self.as_object().clone(),
            PyString::from("__name__").into_ref(vm),
            None,
        )
        .unwrap_or(None)
        .and_then(|obj| {
            obj.payload::<PyString>()
                .map(|s| s.borrow_value().to_owned())
        })
    }

    #[pymethod(magic)]
    fn getattribute(self, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        vm.generic_getattribute_opt(self.as_object().clone(), name.clone(), None)?
            .ok_or_else(|| {
                let module_name = if let Some(name) = self.name(vm) {
                    format!(" '{}'", name)
                } else {
                    "".to_owned()
                };
                vm.new_attribute_error(
                    format!("module{} has no attribute '{}'", module_name, name,),
                )
            })
    }

    #[pymethod(magic)]
    fn repr(self, vm: &VirtualMachine) -> PyResult {
        let importlib = vm.import("_frozen_importlib", &[], 0)?;
        let module_repr = vm.get_attribute(importlib, "_module_repr")?;
        vm.invoke(&module_repr, vec![self.into_object()])
    }

    #[pymethod(magic)]
    fn dir(self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let dict = self
            .as_object()
            .dict()
            .ok_or_else(|| vm.new_value_error("module has no dict".to_owned()))?;
        let attrs = dict.into_iter().map(|(k, _v)| k).collect();
        Ok(vm.ctx.new_list(attrs))
    }
}

pub(crate) fn init(context: &PyContext) {
    PyModuleRef::extend_class(&context, &context.types.module_type);
}
