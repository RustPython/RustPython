/*! The python `frame` type.

*/

use super::objcode::PyCodeRef;
use super::objdict::PyDictRef;
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
    fn f_lineno(self) -> usize {
        self.current_location().row()
    }

    #[pyproperty]
    fn f_trace(self, vm: &VirtualMachine) -> PyObjectRef {
        let result = self.trace.clone().unwrap_or_else(|| vm.get_none());
        println!("{:#?}", result);
        result
    }

    #[pyproperty(setter)]
    fn set_f_trace(self, value: PyObjectRef, vm: &VirtualMachine) {
        println!("value={:#?}", value);
        let trace = &self.trace;
        println!("trace={:#?}", trace);
        let mut cloned = trace.clone();
        println!("cloned={:#?}", cloned);
        let replaced = cloned.replace(value.clone());
        println!("replaced={:#?}", replaced);
        self.f_trace(vm);
    }
}
