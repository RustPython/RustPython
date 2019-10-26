use crate::frame::FrameRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyValue};
use crate::vm::VirtualMachine;

#[pyclass]
#[derive(Debug)]
pub struct PyTraceback {
    pub next: Option<PyTracebackRef>,
    pub frame: FrameRef,
    pub lasti: usize,
    pub lineno: usize,
}

pub type PyTracebackRef = PyRef<PyTraceback>;

impl PyValue for PyTraceback {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.traceback_type()
    }
}

#[pyimpl]
impl PyTraceback {
    pub fn new(
        next: PyObjectRef,
        frame: FrameRef,
        lasti: usize,
        lineno: usize,
        vm: &VirtualMachine,
    ) -> Self {
        let next = if vm.is_none(&next) {
            None
        } else {
            let traceback: PyTracebackRef =
                next.downcast().expect("next must be a traceback object");
            Some(traceback)
        };

        PyTraceback {
            next,
            frame,
            lasti,
            lineno,
        }
    }

    #[pyproperty(name = "tb_frame")]
    fn frame(&self, _vm: &VirtualMachine) -> FrameRef {
        self.frame.clone()
    }

    #[pyproperty(name = "tb_lasti")]
    fn lasti(&self, _vm: &VirtualMachine) -> usize {
        self.lasti
    }

    #[pyproperty(name = "tb_lineno")]
    fn lineno(&self, _vm: &VirtualMachine) -> usize {
        self.lineno
    }

    #[pyproperty(name = "tb_next")]
    fn next_get(&self, _vm: &VirtualMachine) -> Option<PyTracebackRef> {
        self.next.as_ref().map(|x| x.clone())
    }
}

pub fn init(context: &PyContext) {
    PyTraceback::extend_class(context, &context.types.traceback_type);
}
