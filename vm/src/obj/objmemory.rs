use crate::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload2, PyObjectRef, PyResult, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyMemoryView {
    obj: PyObjectRef,
}

impl PyObjectPayload2 for PyMemoryView {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.memoryview_type()
    }
}

pub fn new_memory_view(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(cls, None), (bytes_object, None)]);
    vm.ctx.set_attr(&cls, "obj", bytes_object.clone());
    Ok(PyObject::new(
        Box::new(PyMemoryView {
            obj: bytes_object.clone(),
        }),
        cls.clone(),
    ))
}

pub fn init(ctx: &PyContext) {
    let memoryview_type = &ctx.memoryview_type;
    ctx.set_attr(
        &memoryview_type,
        "__new__",
        ctx.new_rustfunc(new_memory_view),
    );
}
