/*! The python `frame` type.

*/

use super::objcode::PyCodeRef;
use super::objdict::PyDictRef;
use crate::frame::FrameRef;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    FrameRef::extend_class(context, &context.types.frame_type);
}

#[pyimpl]
impl FrameRef {
    #[pyslot(new)]
    fn tp_new(_cls: FrameRef, vm: &VirtualMachine) -> PyResult<Self> {
        Err(vm.new_type_error("Cannot directly create frame object".to_string()))
    }

    #[pymethod(name = "__repr__")]
    fn repr(self, _vm: &VirtualMachine) -> String {
        "<frame object at .. >".to_string()
    }

    #[pyproperty]
    fn f_globals(self, _vm: &VirtualMachine) -> PyDictRef {
        self.scope.globals.clone()
    }

    #[pyproperty]
    fn f_locals(self, _vm: &VirtualMachine) -> PyDictRef {
        self.scope.get_locals()
    }

    #[pyproperty]
    fn f_code(self, _vm: &VirtualMachine) -> PyCodeRef {
        self.code.clone()
    }

    #[pyproperty]
    fn f_back(self, vm: &VirtualMachine) -> PyObjectRef {
        // TODO: how to retrieve the upper stack frame??
        vm.ctx.none()
    }

    #[pyproperty]
    fn f_lasti(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self.lasti.get())
    }
}
