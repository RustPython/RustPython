use crate::frame::FrameRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyContext, PyRef, PyValue};
use crate::vm::VirtualMachine;

#[pyclass(module = false, name = "traceback")]
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
        vm.ctx.types.traceback_type.clone()
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
    fn frame(&self) -> FrameRef {
        self.frame.clone()
    }

    #[pyproperty(name = "tb_lasti")]
    fn lasti(&self) -> usize {
        self.lasti
    }

    #[pyproperty(name = "tb_lineno")]
    fn lineno(&self) -> usize {
        self.lineno
    }

    #[pyproperty(name = "tb_next")]
    fn next_get(&self) -> Option<PyTracebackRef> {
        self.next.as_ref().cloned()
    }
}

impl PyTracebackRef {
    pub fn iter<'a>(&'a self) -> impl Iterator<Item = PyTracebackRef> + 'a {
        std::iter::successors(Some(self.clone()), |tb| tb.next.clone())
    }
}

pub fn init(context: &PyContext) {
    PyTraceback::extend_class(context, &context.types.traceback_type);
}

impl serde::Serialize for PyTraceback {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let mut struc = s.serialize_struct("PyTraceback", 3)?;
        struc.serialize_field("name", &self.frame.code.obj_name)?;
        struc.serialize_field("lineno", &self.lineno)?;
        struc.serialize_field("filename", &self.frame.code.source_path)?;
        struc.end()
    }
}
