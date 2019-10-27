use crate::frame::FrameRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyContext, PyRef, PyValue};
use crate::vm::VirtualMachine;

#[pyclass]
#[derive(Debug)]
pub struct PyTraceback {
    pub next: Option<PyTracebackRef>, // TODO: Make mutable
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
    pub fn new(next: Option<PyTracebackRef>, frame: FrameRef, lasti: usize, lineno: usize) -> Self {
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
        self.next.as_ref().cloned()
    }
}

pub fn init(context: &PyContext) {
    PyTraceback::extend_class(context, &context.types.traceback_type);
}
