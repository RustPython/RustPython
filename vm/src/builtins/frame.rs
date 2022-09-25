/*! The python `frame` type.

*/

use super::{PyCode, PyDictRef};
use crate::{
    class::PyClassImpl,
    frame::{Frame, FrameRef},
    function::PySetterValue,
    types::{Constructor, Unconstructible},
    AsObject, Context, PyObjectRef, PyRef, PyResult, VirtualMachine,
};

pub fn init(context: &Context) {
    FrameRef::extend_class(context, context.types.frame_type);
}

#[pyclass(with(Constructor, PyRef))]
impl Frame {}
impl Unconstructible for Frame {}

#[pyclass]
impl FrameRef {
    #[pymethod(magic)]
    fn repr(self) -> String {
        "<frame object at .. >".to_owned()
    }

    #[pymethod]
    fn clear(self) {
        // TODO
    }

    #[pygetset]
    fn f_globals(self) -> PyDictRef {
        self.globals.clone()
    }

    #[pygetset]
    fn f_locals(self, vm: &VirtualMachine) -> PyResult {
        self.locals(vm).map(Into::into)
    }

    #[pygetset]
    pub fn f_code(self) -> PyRef<PyCode> {
        self.code.clone()
    }

    #[pygetset]
    pub fn f_back(self, vm: &VirtualMachine) -> Option<Self> {
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

    #[pygetset]
    fn f_lasti(self) -> u32 {
        self.lasti()
    }

    #[pygetset]
    pub fn f_lineno(self) -> usize {
        self.current_location().row()
    }

    #[pygetset]
    fn f_trace(self) -> PyObjectRef {
        let boxed = self.trace.lock();
        boxed.clone()
    }

    #[pygetset(setter)]
    fn set_f_trace(self, value: PySetterValue, vm: &VirtualMachine) {
        let mut storage = self.trace.lock();
        *storage = value.unwrap_or_none(vm);
    }
}
