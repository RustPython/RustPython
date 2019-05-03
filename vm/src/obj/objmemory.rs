use crate::obj::objbyteinner::try_as_byte;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

pub type PyMemoryViewRef = PyRef<PyMemoryView>;

#[derive(Debug)]
pub struct PyMemoryView {
    obj: PyObjectRef,
}

impl PyMemoryView {
    pub fn get_obj_value(&self) -> Option<Vec<u8>> {
        try_as_byte(&self.obj)
    }
}

impl PyValue for PyMemoryView {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.memoryview_type()
    }
}

pub fn new_memory_view(
    cls: PyClassRef,
    bytes_object: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyMemoryViewRef> {
    vm.set_attr(cls.as_object(), "obj", bytes_object.clone())?;
    PyMemoryView { obj: bytes_object }.into_ref_with_type(vm, cls)
}

pub fn init(ctx: &PyContext) {
    let memoryview_type = &ctx.memoryview_type;
    extend_class!(ctx, memoryview_type, {
        "__new__" => ctx.new_rustfunc(new_memory_view)
    });
}
