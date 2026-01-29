use super::PyType;
use crate::{Context, Py, PyPayload, PyResult, class::PyClassImpl, types::Representable};

/// PyCapsule - a container for C pointers.
/// In RustPython, this is a minimal implementation for compatibility.
#[pyclass(module = false, name = "PyCapsule")]
#[derive(Debug, Clone, Copy)]
pub struct PyCapsule {
    // Capsules store opaque pointers; we don't expose the actual pointer functionality
    // since RustPython doesn't have the same C extension model as CPython.
    _private: (),
}

impl PyPayload for PyCapsule {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.capsule_type
    }
}

#[pyclass(with(Representable), flags(DISALLOW_INSTANTIATION))]
impl PyCapsule {}

impl Representable for PyCapsule {
    #[inline]
    fn repr_str(_zelf: &Py<Self>, _vm: &crate::VirtualMachine) -> PyResult<String> {
        Ok("<capsule object>".to_string())
    }
}

pub fn init(context: &Context) {
    PyCapsule::extend_class(context, context.types.capsule_type);
}
