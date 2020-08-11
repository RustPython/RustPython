/*! The python `frame` type.

*/

use super::objcode::PyCodeRef;
use super::objdict::PyDictRef;
use super::objstr::PyStringRef;
use crate::frame::FrameRef;
use crate::pyobject::{IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    FrameRef::extend_class(context, &context.types.frame_type);
}

#[pyimpl]
impl FrameRef {
    #[pyslot]
    fn tp_new(_cls: FrameRef, vm: &VirtualMachine) -> PyResult<Self> {
        Err(vm.new_type_error("Cannot directly create frame object".to_owned()))
    }

    #[pymethod(name = "__repr__")]
    fn repr(self) -> String {
        "<frame object at .. >".to_owned()
    }

    #[pymethod(name = "__delattr__")]
    fn delattr(self, value: PyStringRef, vm: &VirtualMachine) {
        // CPython' Frame.f_trace is set to None when deleted.
        // The strange behavior is mimicked here make bdb.py happy about it.
        if value.to_string() == "f_trace" {
            self.set_f_trace(vm.get_none());
        };
    }

    #[pymethod]
    fn clear(self) {
        // TODO
    }

    #[pyproperty]
    fn f_globals(self) -> PyDictRef {
        self.scope.globals.clone()
    }

    #[pyproperty]
    fn f_locals(self) -> PyDictRef {
        self.scope.get_locals()
    }

    #[pyproperty]
    fn f_code(self) -> PyCodeRef {
        self.code.clone()
    }

    #[pyproperty]
    fn f_back(self, vm: &VirtualMachine) -> Option<FrameRef> {
        // TODO: actually store f_back inside Frame struct

        // get the frame in the frame stack that appears before this one.
        // won't work if  this frame isn't in the frame stack, hence the todo above
        vm.frames
            .borrow()
            .iter()
            .rev()
            .skip_while(|p| !p.is(&self))
            .nth(1)
            .cloned()
    }

    #[pyproperty]
    fn f_lasti(self) -> usize {
        self.lasti()
    }

    #[pyproperty]
    pub fn f_lineno(self) -> usize {
        self.current_location().row()
    }

    #[pyproperty]
    fn f_trace(self) -> PyObjectRef {
        let boxed = self.trace.lock();
        boxed.clone()
    }

    #[pyproperty(setter)]
    fn set_f_trace(self, value: PyObjectRef) {
        let mut storage = self.trace.lock();
        *storage = value;
    }
}
