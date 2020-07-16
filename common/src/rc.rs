use std::sync::{Arc, Weak};

// type aliases instead of newtypes because you can't do `fn method(self: PyRc<Self>)` with a
// newtype; requires the arbitrary_self_types unstable feature
pub type PyRc<T> = Arc<T>;
pub type PyWeak<T> = Weak<T>;
