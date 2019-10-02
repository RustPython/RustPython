/*! The python `frame` type.

*/

use super::objcode::PyCodeRef;
use super::objdict::PyDictRef;
use crate::frame::FrameRef;
use crate::pyobject::{PyContext, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    extend_class!(context, &context.types.frame_type, {
        (slot new) => FrameRef::new,
        "__repr__" => context.new_rustfunc(FrameRef::repr),
        "f_locals" => context.new_property(FrameRef::flocals),
        "f_globals" => context.new_property(FrameRef::f_globals),
        "f_code" => context.new_property(FrameRef::fcode),
        "f_back" => context.new_property(FrameRef::f_back),
        "f_lasti" => context.new_property(FrameRef::f_lasti),
    });
}

impl FrameRef {
    #[allow(clippy::new_ret_no_self)]
    fn new(_class: FrameRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot directly create frame object".to_string()))
    }

    fn repr(self, _vm: &VirtualMachine) -> String {
        "<frame object at .. >".to_string()
    }

    fn f_globals(self, _vm: &VirtualMachine) -> PyDictRef {
        self.scope.globals.clone()
    }

    fn flocals(self, _vm: &VirtualMachine) -> PyDictRef {
        self.scope.get_locals()
    }

    fn fcode(self, vm: &VirtualMachine) -> PyCodeRef {
        vm.ctx.new_code_object(self.code.clone())
    }

    fn f_back(self, vm: &VirtualMachine) -> PyObjectRef {
        // TODO: how to retrieve the upper stack frame??
        vm.ctx.none()
    }

    fn f_lasti(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(*self.lasti.borrow())
    }
}
