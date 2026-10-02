use super::{PyStrRef, PyType, union_};
use crate::{
    Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    object::{PyAtomicRef, Traverse, TraverseFn},
    protocol::PyNumberMethods,
    types::{AsNumber, Constructor, Representable},
};

/// Create a unique sentinel object with the given name.
#[pyclass(module = false, name = "sentinel", traverse = "manual")]
#[derive(Debug)]
pub struct PySentinel {
    #[pymember(name = "__name__", type = "object_ex")]
    name: PyStrRef,
    #[pymember(name = "__module__", type = "object_ex", writable)]
    module: PyAtomicRef<Option<PyObject>>,
    repr: Option<PyStrRef>,
}

// SAFETY: Visit each owned Python reference once, without cloning it.
unsafe impl Traverse for PySentinel {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.name.traverse(tracer_fn);
        self.module.traverse(tracer_fn);
        self.repr.traverse(tracer_fn);
    }

    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        // String subclasses may own references back to this sentinel.
        let name = core::mem::replace(&mut self.name, Context::genesis().empty_str.to_owned());
        out.push(name.into());
        if let Some(module) = self.module.store(None) {
            out.push(module);
        }
        if let Some(repr) = self.repr.take() {
            out.push(repr.into());
        }
    }
}

impl PyPayload for PySentinel {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.sentinel_type
    }
}

#[derive(FromArgs)]
pub struct SentinelArgs {
    #[pyarg(positional, error_msg = "sentinel() argument 1 must be str")]
    name: PyStrRef,
    #[pyarg(
        named,
        optional,
        error_msg = "sentinel() argument 'repr' must be str or None"
    )]
    repr: Option<PyStrRef>,
}

impl Constructor for PySentinel {
    type Args = SentinelArgs;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        let module = if let Some(frame) = vm.current_frame()
            && let Some(function) = frame.iframe().func_obj()
        {
            // Use the module captured by the caller function, including the
            // synthetic functions used for module, eval, and exec code.
            function.get_attr(identifier!(vm, __module__), vm)?
        } else {
            vm.ctx.none()
        };
        Ok(Self {
            name: args.name,
            module: PyAtomicRef::from(Some(module)),
            repr: args.repr,
        })
    }
}

#[pyclass(with(Constructor, Representable, AsNumber), flags(HAVE_GC))]
impl PySentinel {
    #[pymethod]
    fn __copy__(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod]
    fn __deepcopy__(zelf: PyRef<Self>, _memo: PyObjectRef) -> PyRef<Self> {
        zelf
    }

    #[pymethod]
    fn __reduce__(zelf: &Py<Self>) -> PyStrRef {
        zelf.name.clone()
    }
}

impl Representable for PySentinel {
    fn repr(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyStrRef> {
        Ok(zelf.repr.as_ref().unwrap_or(&zelf.name).clone())
    }
}

impl AsNumber for PySentinel {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            or: Some(|a, b, vm| union_::or_op(a.to_owned(), b.to_owned(), vm)),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

pub(crate) fn init(context: &'static Context) {
    PySentinel::extend_class(context, context.types.sentinel_type);
}
