use core::ptr::{self, NonNull};
pub use core::sync::atomic::*;
pub use radium::Radium;
use std::{marker::PhantomData, mem::transmute};

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

pub trait StructConverter {
    type T: Sized;
    fn save(val: Self::T) -> usize;
    fn restore(mem: usize) -> Self::T;
}

pub struct PyAtomicStruct<W: StructConverter> {
    inner: PyAtomic<usize>,
    _marker: PhantomData<W>,
}

impl<W: StructConverter> PyAtomicStruct<W> {
    pub fn new(val: W::T) -> Self {
        Self {
            inner: Radium::new(W::save(val)),
            _marker: Default::default(),
        }
    }

    pub fn load(&self, order: Ordering) -> W::T {
        W::restore(self.inner.load(order))
    }

    pub fn store(&self, val: W::T, order: Ordering) {
        self.inner.store(W::save(val), order)
    }
}

#[macro_export]
macro_rules! atomic_struct_transmute {
    ($($vis:vis type $name:ident: $typ:ty;)*) => {
        $(rustpython_vm::__exports::paste::paste! {
            struct [<$name Wrapper>]($typ);
            impl rustpython_common::atomic::StructConverter for [<$name Wrapper>] {
                type T = $typ;
                fn save(x: Self::T) -> usize {
                    unsafe { std::mem::transmute(x) }
                }
                fn restore(x: usize) -> Self::T {
                    unsafe { std::mem::transmute(x) }
                }
            }

            $vis type $name = rustpython_common::atomic::PyAtomicStruct::<[<$name Wrapper>]>;
        })*
    }
}
pub use atomic_struct_transmute;

pub trait FnPtr: Copy + sealed::Sealed {}

pub struct PyAtomicFn<T: FnPtr> {
    inner: PyAtomic<*mut u8>,
    _marker: PhantomData<T>,
}

impl<T: FnPtr> Clone for PyAtomicFn<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Radium::new(self.inner.load(Ordering::Relaxed)),
            _marker: self._marker.clone(),
        }
    }
}

impl<T: FnPtr> PyAtomicFn<T> {
    pub fn new(ptr: T) -> Self {
        Self {
            inner: Radium::new(unsafe { transmute(&ptr) }),
            _marker: PhantomData::default(),
        }
    }

    pub fn load(&self, order: Ordering) -> T {
        unsafe { *transmute::<_, &T>(self.inner.load(order)) }
    }

    pub fn store(&self, ptr: T, order: Ordering) {
        unsafe {
            self.inner.store(transmute(&ptr), order);
        }
    }
}

macro_rules! impl_fn_ptr {
    ($($arg:ident),*) => {
        impl<Ret, $($arg),*> sealed::Sealed for fn($($arg),*) -> Ret {}
        impl<Ret, $($arg),*> FnPtr for fn($($arg),*) -> Ret {}
        impl<Ret, $($arg),*> sealed::Sealed for Option<fn($($arg),*) -> Ret> {}
        impl<Ret, $($arg),*> FnPtr for Option<fn($($arg),*) -> Ret> {}
    };
}

impl_fn_ptr!();
impl_fn_ptr!(A);
impl_fn_ptr!(A, B);
impl_fn_ptr!(A, B, C);
impl_fn_ptr!(A, B, C, D);
impl_fn_ptr!(A, B, C, D, E);
impl_fn_ptr!(A, B, C, D, E, F);
impl_fn_ptr!(A, B, C, D, E, F, G);
impl_fn_ptr!(A, B, C, D, E, F, G, H);
impl_fn_ptr!(A, B, C, D, E, F, G, H, I);
impl_fn_ptr!(A, B, C, D, E, F, G, H, I, J);
impl_fn_ptr!(A, B, C, D, E, F, G, H, I, J, K);
impl_fn_ptr!(A, B, C, D, E, F, G, H, I, J, K, L);

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
