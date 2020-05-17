use super::objbyteinner::try_as_byte;
use super::objtype::{issubclass, PyClassRef};
use crate::pyobject::{
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::stdlib::array::PyArray;
use crate::vm::VirtualMachine;

#[pyclass(name = "memoryview")]
#[derive(Debug)]
pub struct PyMemoryView {
    obj_ref: PyObjectRef,
}

pub type PyMemoryViewRef = PyRef<PyMemoryView>;

#[pyimpl]
impl PyMemoryView {
    pub fn try_value(&self) -> Option<Vec<u8>> {
        try_as_byte(self.obj_ref.clone())
    }

    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        bytes_object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyMemoryViewRef> {
        let object_type = bytes_object.class();

        if issubclass(&object_type, &vm.ctx.types.memoryview_type)
            || issubclass(&object_type, &vm.ctx.types.bytes_type)
            || issubclass(&object_type, &vm.ctx.types.bytearray_type)
            || issubclass(&object_type, &PyArray::class(vm))
        {
            PyMemoryView {
                obj_ref: bytes_object.clone(),
            }
            .into_ref_with_type(vm, cls)
        } else {
            Err(vm.new_type_error(format!(
                "memoryview: a bytes-like object is required, not '{}'",
                object_type.name
            )))
        }
    }

    #[pyproperty]
    fn obj(&self, __vm: &VirtualMachine) -> PyObjectRef {
        self.obj_ref.clone()
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self, vm: &VirtualMachine) -> PyResult {
        vm.call_method(&self.obj_ref, "__hash__", vec![])
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm.call_method(&self.obj_ref, "__getitem__", vec![needle])
    }

    #[pymethod(magic)]
    fn len(&self, vm: &VirtualMachine) -> PyResult {
        vm.call_method(&self.obj_ref, "__len__", vec![])
    }
}

impl PyValue for PyMemoryView {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.memoryview_type()
    }
}

pub(crate) fn init(ctx: &PyContext) {
    PyMemoryView::extend_class(ctx, &ctx.types.memoryview_type)
}
