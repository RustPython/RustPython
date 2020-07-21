#[cfg(not(feature = "threading"))]
use std::rc::{Rc, Weak};
#[cfg(feature = "threading")]
use std::sync::{Arc, Weak};

// type aliases instead of newtypes because you can't do `fn method(self: PyRc<Self>)` with a
// newtype; requires the arbitrary_self_types unstable feature

#[cfg(feature = "threading")]
pub type PyRc<T> = Arc<T>;
#[cfg(not(feature = "threading"))]
pub type PyRc<T> = Rc<T>;

pub type PyWeak<T> = Weak<T>;
