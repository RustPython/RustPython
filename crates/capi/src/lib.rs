use crate::pystate::with_vm;
pub use rustpython_vm::PyObject;

extern crate alloc;

pub mod abstract_;
pub mod bytesobject;
pub mod ceval;
pub mod dictobject;
pub mod import;
pub mod longobject;
pub mod object;
pub mod pyerrors;
pub mod pylifecycle;
pub mod pystate;
pub mod refcount;
pub mod traceback;
pub mod tupleobject;
pub mod unicodeobject;
pub(crate) mod util;

#[inline]
pub(crate) fn log_stub(name: &str) {
    eprintln!("[rustpython-capi stub] {name} called");
}
