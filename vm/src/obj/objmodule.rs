use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyContext, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyModule {
    pub name: String,
}
pub type PyModuleRef = PyRef<PyModule>;

impl PyValue for PyModule {
    const HAVE_DICT: bool = true;

    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.module_type()
    }
}

impl PyModuleRef {
    fn new(cls: PyClassRef, name: PyStringRef, vm: &VirtualMachine) -> PyResult<PyModuleRef> {
        let zelf = PyModule {
            name: name.as_str().to_owned(),
        }
        .into_ref_with_type(vm, cls)?;
        vm.set_attr(zelf.as_object(), "__name__", name)?;
        Ok(zelf)
    }

    fn getattribute(self, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        vm.generic_getattribute(self.as_object().clone(), name.clone())?
            .ok_or_else(|| {
                vm.new_attribute_error(format!(
                    "module '{}' has no attribute '{}'",
                    self.name, name,
                ))
            })
    }

    fn repr(self, vm: &VirtualMachine) -> PyResult {
        let importlib = vm.import("_frozen_importlib", &vm.ctx.new_tuple(vec![]), 0)?;
        let module_repr = vm.get_attribute(importlib, "_module_repr")?;
        vm.invoke(&module_repr, vec![self.into_object()])
    }
}

pub fn init(context: &PyContext) {
    extend_class!(&context, &context.types.module_type, {
        "__new__" => context.new_rustfunc(PyModuleRef::new),
        "__getattribute__" => context.new_rustfunc(PyModuleRef::getattribute),
        "__repr__" => context.new_rustfunc(PyModuleRef::repr),
    });
}
