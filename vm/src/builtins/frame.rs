/*! The python `frame` type.

*/

use super::{PyCode, PyDictRef, PyStrRef, PyTypeRef};
use crate::{
    frame::{Frame, FrameRef},
    function::FuncArgs,
    IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, VirtualMachine,
};

pub fn init(context: &PyContext) {
    FrameRef::extend_class(context, &context.types.frame_type);
}

#[pyimpl(with(PyRef))]
impl Frame {}

#[pyimpl]
impl FrameRef {
    #[pyslot]
    fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot directly create frame object".to_owned()))
    }

    #[pymethod(magic)]
    fn repr(self) -> String {
        "<frame object at .. >".to_owned()
    }

    #[pymethod(magic)]
    fn delattr(self, value: PyStrRef, vm: &VirtualMachine) {
        // CPython' Frame.f_trace is set to None when deleted.
        // The strange behavior is mimicked here make bdb.py happy about it.
        if value.to_string() == "f_trace" {
            self.set_f_trace(vm.ctx.none());
        };
    }

    #[pymethod]
    fn clear(self) {
        // TODO
    }

    #[pyproperty]
    fn f_globals(self) -> PyDictRef {
        self.globals.clone()
    }

    #[pyproperty]
    fn f_locals(self, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        self.locals(vm)
    }

    #[pyproperty]
    fn f_code(self) -> PyRef<PyCode> {
        self.code.clone()
    }

    #[pyproperty]
    fn f_back(self, vm: &VirtualMachine) -> Option<Self> {
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
    fn f_lasti(self) -> u32 {
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
