use crate::obj::objbyteinner::try_as_byte;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[pyclass(name = "memoryview")]
#[derive(Debug)]
pub struct PyMemoryView {
    obj_ref: PyObjectRef,
}

pub type PyMemoryViewRef = PyRef<PyMemoryView>;

#[pyimpl]
impl PyMemoryView {
    pub fn get_obj_value(&self) -> Option<Vec<u8>> {
        try_as_byte(&self.obj_ref)
    }

    #[pyslot(new)]
    fn tp_new(
        cls: PyClassRef,
        bytes_object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyMemoryViewRef> {
        PyMemoryView {
            obj_ref: bytes_object.clone(),
        }
        .into_ref_with_type(vm, cls)
    }

    #[pyproperty]
    fn obj(&self, __vm: &VirtualMachine) -> PyObjectRef {
        self.obj_ref.clone()
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm.call_method(&self.obj_ref, "__getitem__", vec![needle])
    }
}

impl PyValue for PyMemoryView {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.memoryview_type()
    }
}

pub fn init(ctx: &PyContext) {
    PyMemoryView::extend_class(ctx, &ctx.types.memoryview_type)
}
