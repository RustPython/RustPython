// type aliases instead of new-types because you can't do `fn method(self: PyRc<Self>)` with a
// newtype; requires the arbitrary_self_types unstable feature

pub type PyRc<T> = cfg_select! {
    feature = "threading" => alloc::sync::Arc::<T>,
    _ => alloc::rc::Rc::<T>,
};
