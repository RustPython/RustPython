use super::{PyCode, PyGenericAlias, PyStrRef, PyTupleRef, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    coroutine::{Coro, warn_deprecated_throw_signature},
    frame::FrameObjectRef,
    function::OptionalArg,
    object::{Traverse, TraverseFn},
    protocol::PyIterReturn,
    types::{Destructor, IterNext, Iterable, Representable, SelfIter},
};
use crossbeam_utils::atomic::AtomicCell;

#[pyclass(module = false, name = "coroutine", traverse = "manual")]
#[derive(Debug)]
// PyCoro_Type in CPython
pub struct PyCoroutine {
    inner: Coro,
    origin: Option<PyTupleRef>,
}

unsafe impl Traverse for PyCoroutine {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.inner.traverse(tracer_fn);
        if let Some(origin) = &self.origin {
            origin.traverse(tracer_fn);
        }
    }
}

impl PyPayload for PyCoroutine {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.coroutine_type
    }
}

#[pyclass(
    flags(DISALLOW_INSTANTIATION, HAS_WEAKREF),
    with(Py, Representable, Destructor)
)]
impl PyCoroutine {
    pub const fn as_coro(&self) -> &Coro {
        &self.inner
    }

    #[must_use]
    pub fn new(
        frame: FrameObjectRef,
        name: PyStrRef,
        qualname: PyStrRef,
        origin: Option<PyTupleRef>,
    ) -> Self {
        Self {
            inner: Coro::new(frame, name, qualname),
            origin,
        }
    }

    #[pygetset]
    /// name of the coroutine
    fn __name__(&self) -> PyStrRef {
        self.inner.name()
    }

    #[pygetset(setter)]
    fn set___name__(&self, name: PyStrRef) {
        self.inner.set_name(name)
    }

    #[pygetset]
    /// qualified name of the coroutine
    fn __qualname__(&self) -> PyStrRef {
        self.inner.qualname()
    }

    #[pygetset(setter)]
    fn set___qualname__(&self, qualname: PyStrRef) {
        self.inner.set_qualname(qualname)
    }

    #[pymethod(name = "__await__")]
    fn r#await(zelf: PyRef<Self>) -> PyCoroutineWrapper {
        PyCoroutineWrapper {
            coro: zelf,
            closed: AtomicCell::new(false),
        }
    }

    #[pygetset]
    fn cr_await(&self, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.inner.frame().yield_from_target()
    }
    #[pygetset]
    fn cr_frame(&self, _vm: &VirtualMachine) -> Option<FrameObjectRef> {
        if self.inner.closed() {
            None
        } else {
            Some(self.inner.frame())
        }
    }
    #[pygetset]
    fn cr_running(&self, _vm: &VirtualMachine) -> bool {
        self.inner.running()
    }
    #[pygetset]
    fn cr_code(&self, _vm: &VirtualMachine) -> PyRef<PyCode> {
        self.inner.frame().iframe().code().to_owned()
    }
    #[pygetset]
    fn cr_origin(&self, _vm: &VirtualMachine) -> Option<PyTupleRef> {
        self.origin.clone()
    }
    #[pygetset]
    fn cr_suspended(&self, _vm: &VirtualMachine) -> bool {
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
impl Py<PyCoroutine> {
    #[pymethod]
    /// send(arg) -> send 'arg' into coroutine,
    /// return next iterated value or raise StopIteration.
    fn send(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        self.inner.send(self.as_object(), value, vm)
    }

    #[pymethod]
    /// throw(value)
    /// throw(type[,value[,traceback]])
    ///
    /// Raise exception in coroutine, return next iterated value or raise
    /// StopIteration.
    /// the (type, val, tb) signature is deprecated,
    /// and may be removed in a future version of Python.
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
    /// close() -> raise GeneratorExit inside coroutine.
    fn close(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.inner.close(self.as_object(), vm)
    }
}

impl Representable for PyCoroutine {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        Ok(zelf.inner.repr(zelf.as_object(), zelf.get_id(), vm))
    }
}

impl Destructor for PyCoroutine {
    fn del(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
        if zelf.inner.closed() || zelf.inner.running() {
            return Ok(());
        }
        if zelf.inner.frame().lasti() == 0 {
            crate::warn::warn_unawaited_coroutine(zelf.as_object(), &zelf.inner.qualname(), vm);
            zelf.inner.closed.store(true);
            return Ok(());
        }
        if let Err(e) = zelf.inner.close(zelf.as_object(), vm) {
            vm.run_unraisable(e, None, zelf.as_object().to_owned());
        }
        Ok(())
    }
}

#[pyclass(module = false, name = "coroutine_wrapper", traverse = "manual")]
#[derive(Debug)]
// PyCoroWrapper_Type in CPython
pub(crate) struct PyCoroutineWrapper {
    coro: PyRef<PyCoroutine>,
    closed: AtomicCell<bool>,
}

unsafe impl Traverse for PyCoroutineWrapper {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.coro.traverse(tracer_fn);
    }
}

impl PyPayload for PyCoroutineWrapper {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.coroutine_wrapper_type
    }
}

#[pyclass(with(IterNext, Iterable))]
impl PyCoroutineWrapper {
    fn check_closed(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.closed.load() {
            return Err(vm.new_runtime_error("cannot reuse already awaited coroutine"));
        }
        Ok(())
    }

    #[pymethod]
    fn send(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        self.check_closed(vm)?;
        let result = self.coro.send(val, vm);
        // Mark as closed if exhausted
        if let Ok(PyIterReturn::StopIteration(_)) = &result {
            self.closed.store(true);
        }
        result
    }

    #[pymethod]
    fn throw(
        &self,
        exc_type: PyObjectRef,
        exc_val: OptionalArg,
        exc_tb: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        self.check_closed(vm)?;
        warn_deprecated_throw_signature(&exc_val, &exc_tb, vm)?;
        let result = self.coro.throw(exc_type, exc_val, exc_tb, vm);
        // Mark as closed if exhausted
        if let Ok(PyIterReturn::StopIteration(_)) = &result {
            self.closed.store(true);
        }
        result
    }

    #[pymethod]
    fn close(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.closed.store(true);
        self.coro.close(vm)
    }
}

impl SelfIter for PyCoroutineWrapper {}
impl IterNext for PyCoroutineWrapper {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        Self::send(zelf, vm.ctx.none(), vm)
    }
}

impl Drop for PyCoroutine {
    fn drop(&mut self) {
        self.inner.frame().clear_generator();
    }
}

pub(crate) fn init(ctx: &'static Context) {
    PyCoroutine::extend_class(ctx, ctx.types.coroutine_type);
    PyCoroutineWrapper::extend_class(ctx, ctx.types.coroutine_wrapper_type);
}
