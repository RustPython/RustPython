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
    name: AtomicPtr<core::ffi::c_char>,
    destructor: AtomicPtr<c_void>,
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
        name: Option<&'static CStr>,
        destructor: Option<unsafe extern "C" fn(_: *mut PyObject)>,
    ) -> Self {
        let name_ptr = name.map_or(core::ptr::null_mut(), |c| c.as_ptr().cast_mut());
        let destructor_ptr = destructor.map_or(core::ptr::null_mut(), |d| d as *mut c_void);
        Self {
            ptr: ptr.into(),
            context: core::ptr::null_mut::<c_void>().into(),
            name: name_ptr.into(),
            destructor: destructor_ptr.into(),
        }
    }

    pub fn pointer(&self) -> *mut c_void {
        self.ptr.load(core::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_pointer(&self, pointer: *mut c_void) {
        self.ptr
            .store(pointer, core::sync::atomic::Ordering::Relaxed);
    }

    pub fn context(&self) -> *mut c_void {
        self.context.load(core::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_context(&self, context: *mut c_void) {
        self.context
            .store(context, core::sync::atomic::Ordering::Relaxed);
    }

    pub fn name(&self) -> Option<&CStr> {
        let ptr = self.name.load(core::sync::atomic::Ordering::Relaxed);
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { CStr::from_ptr(ptr) })
        }
    }

    pub fn set_name(&self, name: Option<&'static CStr>) {
        let ptr = name.map_or(core::ptr::null_mut(), |c| c.as_ptr().cast_mut());
        self.name.store(ptr, core::sync::atomic::Ordering::Relaxed);
    }

    pub fn destructor(&self) -> Option<unsafe extern "C" fn(_: *mut PyObject)> {
        let ptr = self.destructor.load(core::sync::atomic::Ordering::Relaxed);
        if ptr.is_null() {
            None
        } else {
            Some(unsafe {
                core::mem::transmute::<*mut c_void, unsafe extern "C" fn(_: *mut PyObject)>(ptr)
            })
        }
    }

    pub fn set_destructor(&self, destructor: Option<unsafe extern "C" fn(_: *mut PyObject)>) {
        let ptr = destructor.map_or(core::ptr::null_mut(), |d| d as *mut c_void);
        self.destructor
            .store(ptr, core::sync::atomic::Ordering::Relaxed);
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
