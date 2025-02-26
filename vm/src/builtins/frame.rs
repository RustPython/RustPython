/*! The python `frame` type.

*/

use super::{PyCode, PyDictRef, PyIntRef, PyStrRef};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    frame::{Frame, FrameRef},
    function::PySetterValue,
    types::{Representable, Unconstructible},
};
use num_traits::Zero;

pub fn init(context: &Context) {
    Frame::extend_class(context, context.types.frame_type);
}

impl Unconstructible for Frame {}

impl Representable for Frame {
    #[inline]
    fn repr(_zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        const REPR: &str = "<frame object at .. >";
        Ok(vm.ctx.intern_str(REPR).to_owned())
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

#[pyclass(with(Unconstructible, Py))]
impl Frame {
    #[pymethod]
    fn clear(&self) {
        // TODO
    }

    #[pygetset]
    fn f_globals(&self) -> PyDictRef {
        self.globals.clone()
    }

    #[pygetset]
    fn f_locals(&self, vm: &VirtualMachine) -> PyResult {
        self.locals(vm).map(Into::into)
    }

    #[pygetset]
    pub fn f_code(&self) -> PyRef<PyCode> {
        self.code.clone()
    }

    #[pygetset]
    fn f_lasti(&self) -> u32 {
        self.lasti()
    }

    #[pygetset]
    pub fn f_lineno(&self) -> usize {
        self.current_location().row.to_usize()
    }

    #[pygetset]
    fn f_trace(&self) -> PyObjectRef {
        let boxed = self.trace.lock();
        boxed.clone()
    }

    #[pygetset(setter)]
    fn set_f_trace(&self, value: PySetterValue, vm: &VirtualMachine) {
        let mut storage = self.trace.lock();
        *storage = value.unwrap_or_none(vm);
    }

    #[pymember(type = "bool")]
    fn f_trace_lines(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf: FrameRef = zelf.downcast().unwrap_or_else(|_| unreachable!());

        let boxed = zelf.trace_lines.lock();
        Ok(vm.ctx.new_bool(*boxed).into())
    }

    #[pymember(type = "bool", setter)]
    fn set_f_trace_lines(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        value: PySetterValue,
    ) -> PyResult<()> {
        match value {
            PySetterValue::Assign(value) => {
                let zelf: FrameRef = zelf.downcast().unwrap_or_else(|_| unreachable!());

                let value: PyIntRef = value.downcast().map_err(|_| {
                    vm.new_type_error("attribute value type must be bool".to_owned())
                })?;

                let mut trace_lines = zelf.trace_lines.lock();
                *trace_lines = !value.as_bigint().is_zero();

                Ok(())
            }
            PySetterValue::Delete => {
                Err(vm.new_type_error("can't delete numeric/char attribute".to_owned()))
            }
        }
    }
}

#[pyclass]
impl Py<Frame> {
    #[pygetset]
    pub fn f_back(&self, vm: &VirtualMachine) -> Option<PyRef<Frame>> {
        // TODO: actually store f_back inside Frame struct

        // get the frame in the frame stack that appears before this one.
        // won't work if  this frame isn't in the frame stack, hence the todo above
        vm.frames
            .borrow()
            .iter()
            .rev()
            .skip_while(|p| !p.is(self.as_object()))
            .nth(1)
            .cloned()
    }
}
