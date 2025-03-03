use rustpython_common::lock::PyMutex;

use super::PyType;
use crate::{
    Context, Py, PyPayload, PyRef, class::PyClassImpl, frame::FrameRef, source_code::LineNumber,
};

#[pyclass(module = false, name = "traceback", traverse)]
#[derive(Debug)]
pub struct PyTraceback {
    pub next: PyMutex<Option<PyTracebackRef>>,
    pub frame: FrameRef,
    #[pytraverse(skip)]
    pub lasti: u32,
    #[pytraverse(skip)]
    pub lineno: LineNumber,
}

pub type PyTracebackRef = PyRef<PyTraceback>;

impl PyPayload for PyTraceback {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.traceback_type
    }
}

#[pyclass]
impl PyTraceback {
    pub fn new(next: Option<PyRef<Self>>, frame: FrameRef, lasti: u32, lineno: LineNumber) -> Self {
        PyTraceback {
            next: PyMutex::new(next),
            frame,
            lasti,
            lineno,
        }
    }

    #[pygetset]
    fn tb_frame(&self) -> FrameRef {
        self.frame.clone()
    }

    #[pygetset]
    fn tb_lasti(&self) -> u32 {
        self.lasti
    }

    #[pygetset]
    fn tb_lineno(&self) -> usize {
        self.lineno.to_usize()
    }

    #[pygetset]
    fn tb_next(&self) -> Option<PyRef<Self>> {
        self.next.lock().as_ref().cloned()
    }

    #[pygetset(setter)]
    fn set_tb_next(&self, value: Option<PyRef<Self>>) {
        *self.next.lock() = value;
    }
}

impl PyTracebackRef {
    pub fn iter(&self) -> impl Iterator<Item = PyTracebackRef> {
        std::iter::successors(Some(self.clone()), |tb| tb.next.lock().clone())
    }
}

pub fn init(context: &Context) {
    PyTraceback::extend_class(context, context.types.traceback_type);
}

#[cfg(feature = "serde")]
impl serde::Serialize for PyTraceback {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let mut struc = s.serialize_struct("PyTraceback", 3)?;
        struc.serialize_field("name", self.frame.code.obj_name.as_str())?;
        struc.serialize_field("lineno", &self.lineno.get())?;
        struc.serialize_field("filename", self.frame.code.source_path.as_str())?;
        struc.end()
    }
}
