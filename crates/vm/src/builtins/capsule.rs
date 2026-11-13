use super::PyType;
use crate::{
    AsObject, Context, Py, PyObject, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    types::{Destructor, Representable},
};
use core::ffi::{CStr, c_void};
use core::sync::atomic::AtomicPtr;

// In RustPython, this is a minimal implementation for compatibility.
#[pyclass(module = false, name = "PyCapsule")]
#[derive(Debug)]
pub struct PyCapsule {
    ptr: AtomicPtr<c_void>,
    context: AtomicPtr<c_void>,
    name: Option<&'static CStr>,
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
    /// Construct a capsule carrying a native protocol value.
    ///
    /// # Safety
    /// `ptr`, `context` (initially null), and `destructor` must satisfy the
    /// protocol identified by `name` or by the receiving API, including valid
    /// pointed-to data, callback signatures, synchronization and required lifetimes.
    /// This also applies to unnamed capsules. Consumers may retain imported
    /// pointers beyond the capsule lifetime when their protocol permits it.
    pub unsafe fn new(
        ptr: *mut c_void,
        name: Option<&'static CStr>,
        destructor: Option<unsafe extern "C" fn(_: *mut PyObject)>,
    ) -> Self {
        Self {
            ptr: ptr.into(),
            context: core::ptr::null_mut::<c_void>().into(),
            name,
            destructor,
        }
    }

    pub fn pointer(&self) -> *mut c_void {
        self.ptr.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// # Safety
    /// The replacement must preserve the capsule protocol and destructor
    /// invariants, including any previously exported pointer lifetimes.
    pub unsafe fn set_pointer(&self, pointer: *mut c_void) {
        self.ptr
            .store(pointer, core::sync::atomic::Ordering::Relaxed);
    }

    pub fn context(&self) -> *mut c_void {
        self.context.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// # Safety
    /// The context must preserve the capsule protocol and destructor invariants.
    /// It can identify the pointee's type even for an unnamed capsule.
    pub unsafe fn set_context(&self, context: *mut c_void) {
        self.context
            .store(context, core::sync::atomic::Ordering::Relaxed);
    }

    pub fn name(&self) -> Option<&CStr> {
        self.name
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
        if zelf.pointer().is_null() {
            return Ok(());
        }
        if let Some(destructor) = zelf.destructor() {
            unsafe { destructor(zelf.as_object().as_raw().cast_mut()) };
        }
        Ok(())
    }
}

pub(crate) fn init(context: &'static Context) {
    PyCapsule::extend_class(context, context.types.capsule_type);
}
