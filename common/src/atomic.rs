pub use core::sync::atomic::*;
pub use radium::Radium;

mod sealed {
    pub trait Sealed {}
}
pub trait PyAtomicScalar: sealed::Sealed {
    type Radium: Radium<Item = Self>;
}

pub type PyAtomic<T> = <T as PyAtomicScalar>::Radium;

#[cfg(feature = "threading")]
macro_rules! atomic_ty {
    ($i:ty, $atomic:ty) => {
        $atomic
    };
}
#[cfg(not(feature = "threading"))]
macro_rules! atomic_ty {
    ($i:ty, $atomic:ty) => {
        core::cell::Cell<$i>
    };
}
macro_rules! impl_atomic_scalar {
    ($(($i:ty, $atomic:ty),)*) => {
        $(
            impl sealed::Sealed for $i {}
            impl PyAtomicScalar for $i {
                type Radium = atomic_ty!($i, $atomic);
            }
        )*
    };
}
impl_atomic_scalar!(
    (u8, AtomicU8),
    (i8, AtomicI8),
    (u16, AtomicU16),
    (i16, AtomicI16),
    (u32, AtomicU32),
    (i32, AtomicI32),
    (u64, AtomicU64),
    (i64, AtomicI64),
    (usize, AtomicUsize),
    (isize, AtomicIsize),
    (bool, AtomicBool),
);

impl<T> sealed::Sealed for *mut T {}
impl<T> PyAtomicScalar for *mut T {
    type Radium = atomic_ty!(*mut T, AtomicPtr<T>);
}
