use rustpython_common::lock::PyMutex;
use std::ops::Deref;

use super::{PyType, PyTypeRef};
use crate::{
    Context, Py, PyPayload, PyRef, PyResult, VirtualMachine, class::PyClassImpl, frame::FrameRef,
    source::LineNumber, types::Constructor,
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
        lineno: LineNumber,
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

    #[pygetset(setter)]
    fn set_tb_next(&self, value: Option<PyRef<Self>>, vm: &VirtualMachine) -> PyResult<()> {
        // Check for circular references using Floyd's cycle detection algorithm
        if let Some(ref _new_tb) = value {
            // Temporarily make the assignment to simulate the new chain
            let old_next = self.next.lock().clone();
            *self.next.lock() = value.clone();
            
            // Use Floyd's cycle detection on the chain starting from self
            let has_cycle = Self::has_cycle_from(self);
            
            // Restore the original state
            *self.next.lock() = old_next;
            
            if has_cycle {
                return Err(vm.new_value_error("circular reference in traceback chain".to_owned()));
            }
        }
        
        *self.next.lock() = value;
        Ok(())
    }
    
    /// Detect cycles in traceback chain using Floyd's cycle detection algorithm
    fn has_cycle_from(start: &PyTraceback) -> bool {
        let mut slow = start.tb_next();
        let mut fast = start.tb_next();
        
        while let (Some(slow_tb), Some(fast_tb)) = (&slow, &fast) {
            // Move slow pointer one step
            slow = slow_tb.tb_next();
            
            // Move fast pointer two steps
            fast = fast_tb.tb_next();
            if let Some(ref fast_tb2) = fast {
                fast = fast_tb2.tb_next();
            } else {
                break;
            }
            
            // Check if slow and fast pointers meet (cycle detected)
            if let (Some(slow_ptr), Some(fast_ptr)) = (&slow, &fast) {
                if std::ptr::eq(
                    slow_ptr.deref().deref() as *const PyTraceback,
                    fast_ptr.deref().deref() as *const PyTraceback
                ) {
                    return true;
                }
            }
        }
        
        false
    }
}

impl Constructor for PyTraceback {
    type Args = (Option<PyRef<PyTraceback>>, FrameRef, u32, usize);

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let (next, frame, lasti, lineno) = args;
        let lineno = LineNumber::new(lineno)
            .ok_or_else(|| vm.new_value_error("lineno must be positive".to_owned()))?;
        let tb = PyTraceback::new(next, frame, lasti, lineno);
        tb.into_ref_with_type(vm, cls).map(Into::into)
    }
}

impl PyTracebackRef {
    pub fn iter(&self) -> impl Iterator<Item = Self> {
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
