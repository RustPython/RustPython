use crate::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyMemoryView {
    obj: PyObjectRef,
}

impl PyValue for PyMemoryView {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.memoryview_type()
    }
}

pub fn new_memory_view(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(cls, None), (bytes_object, None)]);
    vm.ctx.set_attr(&cls, "obj", bytes_object.clone());
    Ok(PyObject::new(
        PyMemoryView {
            obj: bytes_object.clone(),
        },
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
