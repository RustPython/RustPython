use crate::pystate::with_vm;
pub use rustpython_vm::PyObject;

extern crate alloc;

pub(crate) mod abstract_;
pub(crate) mod bytearrayobject;
pub(crate) mod bytesobject;
pub(crate) mod capsule;
pub(crate) mod ceval;
pub(crate) mod complexobject;
pub(crate) mod dictobject;
pub(crate) mod extension_loader;
pub(crate) mod floatobject;
pub(crate) mod handles;
pub(crate) mod import;
pub(crate) mod listobject;
pub(crate) mod longobject;
pub(crate) mod methodobject;
pub(crate) mod moduleobject;
pub(crate) mod object;
pub(crate) mod objimpl;
pub(crate) mod pybuffer;
pub(crate) mod pyerrors;
pub(crate) mod pylifecycle;
pub(crate) mod pystate;
pub(crate) mod refcount;
pub(crate) mod traceback;
pub(crate) mod tupleobject;
pub(crate) mod unicodeobject;
pub(crate) mod util;

#[inline]
pub(crate) fn log_stub(name: &str) {
    let _ = name;
}

unsafe extern "C" {
    fn RustPython_Keep_PyObject_CallMethodObjArgs() -> *mut core::ffi::c_void;
}

#[used]
static KEEP_PYOBJECT_CALLMETHODOBJARGS: unsafe extern "C" fn() -> *mut core::ffi::c_void =
    RustPython_Keep_PyObject_CallMethodObjArgs;

pub use crate::pylifecycle::initialize_for_vm;
