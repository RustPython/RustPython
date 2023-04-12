mod object;
mod trace;

pub(in crate::object) use object::PyObjVTable;
pub use trace::{MaybeTrace, Trace, TracerFn};
