use super::objdict::PyDictRef;
use super::objstr::{PyString, PyStringRef};
use super::objtype::PyClassRef;
use crate::function::OptionalOption;
use crate::pyobject::{ItemProtocol, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyModule {}
pub type PyModuleRef = PyRef<PyModule>;

impl PyValue for PyModule {
    const HAVE_DICT: bool = true;

    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.module_type()
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

impl PyModuleRef {
    fn new(
        cls: PyClassRef,
        name: PyStringRef,
        doc: OptionalOption<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyModuleRef> {
        let zelf = PyModule {}.into_ref_with_type(vm, cls)?;
        init_module_dict(
            vm,
            &zelf.as_object().dict.as_ref().unwrap().borrow(),
            name.into_object(),
            doc.flat_option()
                .map_or_else(|| vm.get_none(), PyRef::into_object),
        );
        Ok(zelf)
    }

    fn name(self, vm: &VirtualMachine) -> Option<String> {
        vm.generic_getattribute(
            self.as_object().clone(),
            PyString::from("__name__").into_ref(vm),
        )
        .unwrap_or(None)
        .and_then(|obj| obj.payload::<PyString>().map(|s| s.as_str().to_owned()))
    }

    fn getattribute(self, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        vm.generic_getattribute(self.as_object().clone(), name.clone())?
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

    fn repr(self, vm: &VirtualMachine) -> PyResult {
        let importlib = vm.import("_frozen_importlib", &[], 0)?;
        let module_repr = vm.get_attribute(importlib, "_module_repr")?;
        vm.invoke(&module_repr, vec![self.into_object()])
    }
}

pub fn init(context: &PyContext) {
    extend_class!(&context, &context.types.module_type, {
        (slot new) => PyModuleRef::new,
        "__getattribute__" => context.new_rustfunc(PyModuleRef::getattribute),
        "__repr__" => context.new_rustfunc(PyModuleRef::repr),
    });
}
