use super::{PyList, PyType};
use crate::{
    AsObject, Context, Py, PyPayload, PyRef, PyResult, VirtualMachine, class::PyClassImpl,
    frame::FrameRef, function::PySetterValue, types::Constructor,
};
use rustpython_common::lock::PyMutex;
use rustpython_compiler_core::OneIndexed;

#[pyclass(module = false, name = "traceback", traverse)]
#[derive(Debug)]
pub struct PyTraceback {
    pub next: PyMutex<Option<PyTracebackRef>>,
    pub frame: FrameRef,
    #[pytraverse(skip)]
    pub lasti: u32,
    #[pytraverse(skip)]
    pub lineno: OneIndexed,
}

pub type PyTracebackRef = PyRef<PyTraceback>;

impl PyPayload for PyTraceback {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.traceback_type
    }
}

#[pyclass(with(Constructor))]
impl PyTraceback {
    pub const fn new(
        next: Option<PyRef<Self>>,
        frame: FrameRef,
        lasti: u32,
        lineno: OneIndexed,
    ) -> Self {
        Self {
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
    const fn tb_lasti(&self) -> u32 {
        self.lasti
    }

    #[pygetset]
    const fn tb_lineno(&self) -> usize {
        self.lineno.get()
    }

    #[pygetset]
    fn tb_next(&self) -> Option<PyRef<Self>> {
        self.next.lock().as_ref().cloned()
    }

    #[pymethod]
    fn __dir__(&self, vm: &VirtualMachine) -> PyList {
        PyList::from(
            ["tb_frame", "tb_next", "tb_lasti", "tb_lineno"]
                .iter()
                .map(|&s| vm.ctx.new_str(s).into())
                .collect::<Vec<_>>(),
        )
    }

    #[pygetset(setter)]
    fn set_tb_next(
        zelf: &Py<Self>,
        value: PySetterValue<Option<PyRef<Self>>>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let value = match value {
            PySetterValue::Assign(v) => v,
            PySetterValue::Delete => {
                return Err(vm.new_type_error("can't delete tb_next attribute".to_owned()));
            }
        };
        if let Some(ref new_next) = value {
            let mut cursor = new_next.clone();
            loop {
                if cursor.is(zelf) {
                    return Err(vm.new_value_error("traceback loop detected".to_owned()));
                }
                let next = cursor.next.lock().clone();
                match next {
                    Some(n) => cursor = n,
                    None => break,
                }
            }
        }
        *zelf.next.lock() = value;
        Ok(())
    }
}

impl Constructor for PyTraceback {
    type Args = (Option<PyRef<Self>>, FrameRef, u32, usize);

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        let (next, frame, lasti, lineno) = args;
        let lineno = OneIndexed::new(lineno)
            .ok_or_else(|| vm.new_value_error("lineno must be positive".to_owned()))?;
        Ok(Self::new(next, frame, lasti, lineno))
    }
}

impl PyTracebackRef {
    pub fn iter(&self) -> impl Iterator<Item = Self> {
        core::iter::successors(Some(self.clone()), |tb| tb.next.lock().clone())
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
        struc.serialize_field("filename", self.frame.code.source_path().as_str())?;
        struc.end()
    }
}
