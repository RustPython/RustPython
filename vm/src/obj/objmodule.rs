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
    fn init(self, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        vm.set_attr(&self.into_object(), "__name__", name)?;
        Ok(vm.get_none())
    }
}

pub fn init(context: &PyContext) {
    extend_class!(&context, &context.types.module_type, {
        "__init__" => context.new_rustfunc(PyModuleRef::init),
    });
}
