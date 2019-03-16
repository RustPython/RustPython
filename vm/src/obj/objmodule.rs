use crate::obj::objstr::PyStringRef;
use crate::pyobject::{DictProtocol, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[derive(Clone, Debug)]
pub struct PyModule {
    pub name: String,
    pub dict: PyObjectRef,
}
pub type PyModuleRef = PyRef<PyModule>;

impl PyValue for PyModule {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.module_type()
    }
}

impl PyModuleRef {
    fn dir(self: PyModuleRef, vm: &mut VirtualMachine) -> PyResult {
        let keys = self
            .dict
            .get_key_value_pairs()
            .iter()
            .map(|(k, _v)| k.clone())
            .collect();
        Ok(vm.ctx.new_list(keys))
    }

    fn set_attr(self, attr: PyStringRef, value: PyObjectRef, vm: &mut VirtualMachine) {
        self.dict.set_item(&vm.ctx, &attr.value, value)
    }
}

pub fn init(context: &PyContext) {
    extend_class!(&context, &context.module_type, {
        "__dir__" => context.new_rustfunc(PyModuleRef::dir),
        "__setattr__" => context.new_rustfunc(PyModuleRef::set_attr)
    });
}
