use super::PyType;
use crate::{
    AsObject, Context, Py, PyObject, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    types::{Destructor, Representable},
};
use core::ffi::c_void;
use core::sync::atomic::AtomicPtr;

/// PyCapsule - a container for C pointers.
/// In RustPython, this is a minimal implementation for compatibility.
#[pyclass(module = false, name = "PyCapsule")]
#[derive(Debug)]
pub struct PyCapsule {
    ptr: AtomicPtr<c_void>,
    destructor: Option<unsafe extern "C" fn(_: *mut PyObject)>,
}

impl PyPayload for PyCapsule {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.capsule_type
    }
}

#[pyclass(with(Representable, Destructor), flags(DISALLOW_INSTANTIATION))]
impl PyCapsule {
    pub fn new(
        ptr: *mut c_void,
        destructor: Option<unsafe extern "C" fn(_: *mut PyObject)>,
    ) -> Self {
        Self {
            ptr: ptr.into(),
            destructor,
        }
    }

    pub fn pointer(&self) -> *mut c_void {
        self.ptr.load(core::sync::atomic::Ordering::Relaxed)
    }

    fn destructor(&self) -> Option<unsafe extern "C" fn(_: *mut PyObject)> {
        self.destructor
    }
}

impl Representable for PyCapsule {
    #[inline]
    fn repr_str(_zelf: &Py<Self>, _vm: &crate::VirtualMachine) -> PyResult<String> {
        Ok("<capsule object>".to_string())
    }
}

impl Destructor for PyCapsule {
    fn del(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<()> {
        if let Some(destructor) = zelf.destructor() {
            unsafe { destructor(zelf.as_object().as_raw().cast_mut()) };
        }
        Ok(())
    }
}

pub(crate) fn init(context: &'static Context) {
    PyCapsule::extend_class(context, context.types.capsule_type);
}
