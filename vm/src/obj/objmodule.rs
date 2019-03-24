use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{DictProtocol, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyModule {
    pub name: String,
    pub dict: PyObjectRef,
}
pub type PyModuleRef = PyRef<PyModule>;

impl PyValue for PyModule {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.module_type()
    }
}

impl PyModuleRef {
    fn dir(self: PyModuleRef, vm: &VirtualMachine) -> PyResult {
        let keys = self
            .dict
            .get_key_value_pairs()
            .iter()
            .map(|(k, _v)| k.clone())
            .collect();
        Ok(vm.ctx.new_list(keys))
    }

    fn set_attr(self, attr: PyStringRef, value: PyObjectRef, vm: &VirtualMachine) {
        self.dict.set_item(&vm.ctx, &attr.value, value)
    }
}

pub fn init(context: &PyContext) {
    extend_class!(&context, &context.module_type, {
        "__dir__" => context.new_rustfunc(PyModuleRef::dir),
        "__setattr__" => context.new_rustfunc(PyModuleRef::set_attr)
    });
}
