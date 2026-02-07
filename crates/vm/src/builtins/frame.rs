/*! The python `frame` type.

*/

use super::{PyCode, PyDictRef, PyIntRef, PyStrRef};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    frame::{Frame, FrameOwner, FrameRef},
    function::PySetterValue,
    types::Representable,
};
use num_traits::Zero;

pub fn init(context: &Context) {
    Frame::extend_class(context, context.types.frame_type);
}

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

#[pyclass(flags(DISALLOW_INSTANTIATION), with(Py))]
impl Frame {
    #[pygetset]
    fn f_globals(&self) -> PyDictRef {
        self.globals.clone()
    }

    #[pygetset]
    fn f_builtins(&self) -> PyObjectRef {
        self.builtins.clone()
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
        // Return byte offset (each instruction is 2 bytes) for compatibility
        self.lasti() * 2
    }

    #[pygetset]
    pub fn f_lineno(&self) -> usize {
        // If lasti is 0, execution hasn't started yet - use first line number
        // Similar to PyCode_Addr2Line which returns co_firstlineno for addr_q < 0
        if self.lasti() == 0 {
            self.code.first_line_number.map(|n| n.get()).unwrap_or(1)
        } else {
            self.current_location().line.get()
        }
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

                let value: PyIntRef = value
                    .downcast()
                    .map_err(|_| vm.new_type_error("attribute value type must be bool"))?;

                let mut trace_lines = zelf.trace_lines.lock();
                *trace_lines = !value.as_bigint().is_zero();

                Ok(())
            }
            PySetterValue::Delete => Err(vm.new_type_error("can't delete numeric/char attribute")),
        }
    }

    #[pymember(type = "bool")]
    fn f_trace_opcodes(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf: FrameRef = zelf.downcast().unwrap_or_else(|_| unreachable!());
        let trace_opcodes = zelf.trace_opcodes.lock();
        Ok(vm.ctx.new_bool(*trace_opcodes).into())
    }

    #[pymember(type = "bool", setter)]
    fn set_f_trace_opcodes(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        value: PySetterValue,
    ) -> PyResult<()> {
        match value {
            PySetterValue::Assign(value) => {
                let zelf: FrameRef = zelf.downcast().unwrap_or_else(|_| unreachable!());

                let value: PyIntRef = value
                    .downcast()
                    .map_err(|_| vm.new_type_error("attribute value type must be bool"))?;

                let mut trace_opcodes = zelf.trace_opcodes.lock();
                *trace_opcodes = !value.as_bigint().is_zero();

                // TODO: Implement the equivalent of _PyEval_SetOpcodeTrace()

                Ok(())
            }
            PySetterValue::Delete => Err(vm.new_type_error("can't delete numeric/char attribute")),
        }
    }
}

#[pyclass]
impl Py<Frame> {
    #[pymethod]
    // = frame_clear_impl
    fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
        let owner = FrameOwner::from_i8(self.owner.load(core::sync::atomic::Ordering::Acquire));
        match owner {
            FrameOwner::Generator => {
                // Generator frame: check if suspended (lasti > 0 means
                // FRAME_SUSPENDED). lasti == 0 means FRAME_CREATED and
                // can be cleared.
                if self.lasti() != 0 {
                    return Err(vm.new_runtime_error("cannot clear a suspended frame".to_owned()));
                }
            }
            FrameOwner::Thread => {
                // Thread-owned frame: always executing, cannot clear.
                return Err(vm.new_runtime_error("cannot clear an executing frame".to_owned()));
            }
            FrameOwner::FrameObject => {
                // Detached frame: safe to clear.
            }
        }

        // Clear fastlocals
        {
            let mut fastlocals = self.fastlocals.lock();
            for slot in fastlocals.iter_mut() {
                *slot = None;
            }
        }

        // Clear the evaluation stack
        self.clear_value_stack();

        // Clear temporary refs
        self.temporary_refs.lock().clear();

        Ok(())
    }

    #[pygetset]
    fn f_generator(&self) -> Option<PyObjectRef> {
        self.generator.to_owned()
    }

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
