/*
 * The mythical generator.
 */

use super::{PyCode, PyGenericAlias, PyStrRef, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    coroutine::{Coro, warn_deprecated_throw_signature},
    frame::FrameObjectRef,
    function::OptionalArg,
    object::{Traverse, TraverseFn},
    protocol::PyIterReturn,
    types::{Destructor, IterNext, Iterable, Representable, SelfIter},
};

#[pyclass(module = false, name = "generator", traverse = "manual")]
#[derive(Debug)]
pub struct PyGenerator {
    inner: Coro,
}

unsafe impl Traverse for PyGenerator {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.inner.traverse(tracer_fn);
    }
}

impl PyPayload for PyGenerator {
    // Tracked in `make_generator_or_coro`, together with the frame the object
    // is born owning, so the pair costs one trip through the GC's gen0 list
    // instead of two.
    const NEW_REF_UNTRACKED: bool = true;

    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.generator_type
    }
}

#[pyclass(
    flags(DISALLOW_INSTANTIATION, HAS_WEAKREF),
    with(Py, IterNext, Iterable, Representable, Destructor)
)]
impl PyGenerator {
    pub const fn as_coro(&self) -> &Coro {
        &self.inner
    }

    #[must_use]
    pub fn new(frame: FrameObjectRef, name: PyStrRef, qualname: PyStrRef) -> Self {
        Self {
            inner: Coro::new(frame, name, qualname),
        }
    }

    #[pygetset]
    fn __name__(&self) -> PyStrRef {
        self.inner.name()
    }

    #[pygetset(setter)]
    fn set___name__(&self, name: PyStrRef) {
        self.inner.set_name(name)
    }

    #[pygetset]
    fn __qualname__(&self) -> PyStrRef {
        self.inner.qualname()
    }

    #[pygetset(setter)]
    fn set___qualname__(&self, qualname: PyStrRef) {
        self.inner.set_qualname(qualname)
    }

    #[pygetset]
    fn gi_frame(&self, _vm: &VirtualMachine) -> Option<FrameObjectRef> {
        if self.inner.closed() {
            None
        } else {
            Some(self.inner.frame())
        }
    }

    #[pygetset]
    fn gi_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }

    #[pygetset]
    fn gi_code(&self, _vm: &VirtualMachine) -> PyRef<PyCode> {
        self.inner.frame().iframe().code().to_owned()
    }

    #[pygetset]
    fn gi_yieldfrom(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().yield_from_target()
    }

    #[pygetset]
    fn gi_suspended(&self, _vm: &VirtualMachine) -> bool {
        self.inner.suspended()
    }

    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

#[pyclass]
impl Py<PyGenerator> {
    #[pymethod]
    fn send(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        self.inner.send(self.as_object(), value, vm)
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        warn_deprecated_throw_signature(&exc_val, &exc_tb, vm)?;
        self.inner.throw(
            self.as_object(),
            exc_type,
            exc_val.unwrap_or_none(vm),
            exc_tb.unwrap_or_none(vm),
            vm,
        )
    }

    #[pymethod]
    fn close(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.inner.close(self.as_object(), vm)
    }
}

impl Representable for PyGenerator {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        Ok(zelf.inner.repr(zelf.as_object(), zelf.get_id(), vm))
    }
}

impl SelfIter for PyGenerator {}
impl IterNext for PyGenerator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.send(vm.ctx.none(), vm)
    }
}

impl Destructor for PyGenerator {
    fn del(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
        // _PyGen_Finalize: close the generator if it's still suspended
        if zelf.inner.closed() || zelf.inner.running() {
            return Ok(());
        }
        // Generator was never started, just mark as closed
        if zelf.inner.frame().lasti() == 0 {
            zelf.inner.closed.store(true);
            return Ok(());
        }
        // Throw GeneratorExit to run finally blocks
        if let Err(e) = zelf.inner.close(zelf.as_object(), vm) {
            vm.run_unraisable(e, None, zelf.as_object().to_owned());
        }
        Ok(())
    }
}

impl Drop for PyGenerator {
    fn drop(&mut self) {
        self.inner.frame().clear_generator();
    }
}

/// Fast, VM-free check mirroring the read-only-state branches of
/// `<PyGenerator as Destructor>::del`: a generator that's already closed or
/// currently running needs no `close()`-style cleanup, so its `del` slot is a
/// documented no-op. Skipping the call avoids attaching to a VM
/// (`with_vm`) on every generator drop for the common case of a generator
/// consumed to completion.
fn generator_del_needed(zelf: &PyObject) -> bool {
    let zelf: &Py<PyGenerator> = zelf
        .downcast_ref()
        .expect("del_needed is only installed on the generator type");
    !(zelf.inner.closed() || zelf.inner.running())
}

pub(crate) fn init(ctx: &'static Context) {
    PyGenerator::extend_class(ctx, ctx.types.generator_type);
    ctx.types
        .generator_type
        .slots
        .del_needed
        .store(Some(generator_del_needed));
}
