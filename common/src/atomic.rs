use core::ptr::{self, NonNull};
pub use core::sync::atomic::*;
pub use radium::Radium;
use std::marker::PhantomData;

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

pub trait FnPtr: Copy + sealed::Sealed {}

impl<Ret> sealed::Sealed for fn() -> Ret {}
impl<Ret> FnPtr for fn() -> Ret {}
impl<Ret, A> sealed::Sealed for fn(A) -> Ret {}
impl<Ret, A> FnPtr for fn(A) -> Ret {}
impl<Ret, A, B> sealed::Sealed for fn(A, B) -> Ret {}
impl<Ret, A, B> FnPtr for fn(A, B) -> Ret {}
impl<Ret, A, B, C> sealed::Sealed for fn(A, B, C) -> Ret {}
impl<Ret, A, B, C> FnPtr for fn(A, B, C) -> Ret {}

impl<Ret> sealed::Sealed for Option<fn() -> Ret> {}
impl<Ret> FnPtr for Option<fn() -> Ret> {}
impl<Ret, A> sealed::Sealed for Option<fn(A) -> Ret> {}
impl<Ret, A> FnPtr for Option<fn(A) -> Ret> {}
impl<Ret, A, B> sealed::Sealed for Option<fn(A, B) -> Ret> {}
impl<Ret, A, B> FnPtr for Option<fn(A, B) -> Ret> {}
impl<Ret, A, B, C> sealed::Sealed for Option<fn(A, B, C) -> Ret> {}
impl<Ret, A, B, C> FnPtr for Option<fn(A, B, C) -> Ret> {}

pub struct PyAtomicFn<T: FnPtr> {
    inner: PyAtomic<*mut u8>,
    _marker: PhantomData<T>,
}

impl<T: FnPtr> PyAtomicFn<T> {
    pub fn load(&self, order: Ordering) -> T {
        let f = self.inner.load(order);
        unsafe { *((&f) as *const _ as *const T) }
    }

    pub fn store(&self, ptr: T, order: Ordering) {
        unsafe {
            self.inner.store(*((&ptr) as *const T as *const _), order);
        }
    }
}

pub struct OncePtr<T> {
    inner: PyAtomic<*mut T>,
}

impl<T> Default for OncePtr<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> OncePtr<T> {
    #[inline]
    pub fn new() -> Self {
        OncePtr {
            inner: Radium::new(ptr::null_mut()),
        }
    }

    pub fn get(&self) -> Option<NonNull<T>> {
        NonNull::new(self.inner.load(Ordering::Acquire))
    }

    pub fn set(&self, value: NonNull<T>) -> Result<(), NonNull<T>> {
        let exchange = self.inner.compare_exchange(
            ptr::null_mut(),
            value.as_ptr(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        match exchange {
            Ok(_) => Ok(()),
            Err(_) => Err(value),
        }
    }

    pub fn get_or_init<F>(&self, f: F) -> NonNull<T>
    where
        F: FnOnce() -> Box<T>,
    {
        enum Void {}
        match self.get_or_try_init(|| Ok::<_, Void>(f())) {
            Ok(val) => val,
            Err(void) => match void {},
        }
    }

    pub fn get_or_try_init<F, E>(&self, f: F) -> Result<NonNull<T>, E>
    where
        F: FnOnce() -> Result<Box<T>, E>,
    {
        if let Some(val) = self.get() {
            return Ok(val);
        }

        Ok(self.initialize(f()?))
    }

    #[cold]
    fn initialize(&self, val: Box<T>) -> NonNull<T> {
        let ptr = Box::into_raw(val);
        let exchange =
            self.inner
                .compare_exchange(ptr::null_mut(), ptr, Ordering::AcqRel, Ordering::Acquire);
        let ptr = match exchange {
            Ok(_) => ptr,
            Err(winner) => {
                drop(unsafe { Box::from_raw(ptr) });
                winner
            }
        };
        unsafe { NonNull::new_unchecked(ptr) }
    }
}
