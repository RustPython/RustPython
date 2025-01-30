use crate::object::Traverse;
use crate::PyObjectRef;

pub trait Visitor {
    /// Visit a synchronized garbage-collected pointer.
    fn visit_sync<T>(&mut self, gc: &PyObjectRef)
    where
        T: Traverse + Send + Sync + ?Sized;
}
