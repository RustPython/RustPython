mod object;
mod traverse;

pub(in crate::object) use object::PyObjVTable;
pub use traverse::{MaybeTraverse, Traverse, TraverseFn};
