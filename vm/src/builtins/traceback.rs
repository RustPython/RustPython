use super::PyTypeRef;
use crate::{frame::FrameRef, PyClassImpl, PyContext, PyRef, PyValue, VirtualMachine};

#[pyclass(module = false, name = "traceback")]
#[derive(Debug)]
pub struct PyTraceback {
    pub next: Option<PyTracebackRef>, // TODO: Make mutable
    pub frame: FrameRef,
    pub lasti: u32,
    pub lineno: usize,
}

pub type PyTracebackRef = PyRef<PyTraceback>;

impl PyValue for PyTraceback {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.traceback_type
    }
}

#[pyimpl]
impl PyTraceback {
    pub fn new(next: Option<PyRef<Self>>, frame: FrameRef, lasti: u32, lineno: usize) -> Self {
        PyTraceback {
            next,
            frame,
            lasti,
            lineno,
        }
    }

    #[pyproperty]
    fn tb_frame(&self) -> FrameRef {
        self.frame.clone()
    }

    #[pyproperty]
    fn tb_lasti(&self) -> u32 {
        self.lasti
    }

    #[pyproperty]
    fn tb_lineno(&self) -> usize {
        self.lineno
    }

    #[pyproperty]
    fn tb_next(&self) -> Option<PyRef<Self>> {
        self.next.as_ref().cloned()
    }
}

impl PyTracebackRef {
    pub fn iter(&self) -> impl Iterator<Item = PyTracebackRef> {
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
        struc.serialize_field("name", self.frame.code.obj_name.as_str())?;
        struc.serialize_field("lineno", &self.lineno)?;
        struc.serialize_field("filename", self.frame.code.source_path.as_str())?;
        struc.end()
    }
}
