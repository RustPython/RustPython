/*! The python `frame` type.

*/

use super::objcode::PyCodeRef;
use super::objdict::PyDictRef;
use crate::frame::FrameRef;
use crate::pyobject::{PyContext, PyResult};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    extend_class!(context, &context.frame_type, {
        "__new__" => context.new_rustfunc(FrameRef::new),
        "__repr__" => context.new_rustfunc(FrameRef::repr),
        "f_locals" => context.new_property(FrameRef::flocals),
        "f_code" => context.new_property(FrameRef::fcode),
    });
}

impl FrameRef {
    fn new(_class: FrameRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot directly create frame object".to_string()))
    }

    fn repr(self, _vm: &VirtualMachine) -> String {
        "<frame object at .. >".to_string()
    }

    fn flocals(self, _vm: &VirtualMachine) -> PyDictRef {
        self.scope.get_locals()
    }

    fn fcode(self, vm: &VirtualMachine) -> PyCodeRef {
        vm.ctx.new_code_object(self.code.clone())
    }
}
