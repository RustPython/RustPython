#[cfg(feature = "threading")]
mod threading {
    use crate::lock::OnceCell;

    pub struct StaticCell<T: 'static> {
        inner: OnceCell<T>,
    }

    impl<T> StaticCell<T> {
        #[doc(hidden)]
        pub const fn _from_once_cell(inner: OnceCell<T>) -> Self {
            Self { inner }
        }

        pub fn get(&'static self) -> Option<&'static T> {
            self.inner.get()
        }

        pub fn set(&'static self, value: T) -> Result<(), T> {
            self.inner.set(value)
        }

        pub fn get_or_init<F>(&'static self, f: F) -> &'static T
        where
            F: FnOnce() -> T,
        {
            self.inner.get_or_init(f)
        }

        pub fn get_or_try_init<F, E>(&'static self, f: F) -> Result<&'static T, E>
        where
            F: FnOnce() -> Result<T, E>,
        {
            if let Some(val) = self.inner.get() {
                return Ok(val);
            }
            let val = f()?;
            let _ = self.inner.set(val);
            Ok(self.inner.get().unwrap())
        }
    }

    #[macro_export]
    macro_rules! static_cell {
        ($($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty;)+) => {
            $($(#[$attr])*
            $vis static $name: $crate::static_cell::StaticCell<$t> =
                $crate::static_cell::StaticCell::_from_once_cell($crate::lock::OnceCell::new());)+
        };
    }
}
#[cfg(feature = "threading")]
pub use threading::*;

#[cfg(all(not(feature = "threading"), feature = "std"))]
mod non_threading {
    use crate::lock::OnceCell;
    use std::thread::LocalKey;

    pub struct StaticCell<T: 'static> {
        inner: &'static LocalKey<OnceCell<&'static T>>,
    }

    fn leak<T>(x: T) -> &'static T {
        Box::leak(Box::new(x))
    }

    impl<T> StaticCell<T> {
        #[doc(hidden)]
        pub const fn _from_local_key(inner: &'static LocalKey<OnceCell<&'static T>>) -> Self {
            Self { inner }
        }

        pub fn get(&'static self) -> Option<&'static T> {
            self.inner.with(|x| x.get().copied())
        }

        pub fn set(&'static self, value: T) -> Result<(), T> {
            self.inner.with(|x| {
                if x.get().is_some() {
                    Err(value)
                } else {
                    let _ = x.set(leak(value));
                    Ok(())
                }
            })
        }

        pub fn get_or_init<F>(&'static self, f: F) -> &'static T
        where
            F: FnOnce() -> T,
        {
            self.inner.with(|x| *x.get_or_init(|| leak(f())))
        }

        pub fn get_or_try_init<F, E>(&'static self, f: F) -> Result<&'static T, E>
        where
            F: FnOnce() -> Result<T, E>,
        {
            self.inner.with(|x| {
                if let Some(val) = x.get() {
                    Ok(*val)
                } else {
                    let val = leak(f()?);
                    let _ = x.set(val);
                    Ok(val)
                }
            })
        }
    }

    #[macro_export]
    macro_rules! static_cell {
        ($($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty;)+) => {
            $($(#[$attr])*
            $vis static $name: $crate::static_cell::StaticCell<$t> = {
                ::std::thread_local! {
                     $vis static $name: $crate::lock::OnceCell<&'static $t> = const {
                         $crate::lock::OnceCell::new()
                     };
                }
                $crate::static_cell::StaticCell::_from_local_key(&$name)
            };)+
        };
    }
}
#[cfg(all(not(feature = "threading"), feature = "std"))]
pub use non_threading::*;

// Same as `threading` variant, but wraps unsync::OnceCell with Sync.
#[cfg(all(not(feature = "threading"), not(feature = "std")))]
mod no_std {
    use crate::lock::OnceCell;

    // unsync::OnceCell is !Sync, but without std there can be no threads.
    struct SyncOnceCell<T>(OnceCell<T>);
    // SAFETY: Without std, threading is impossible.
    unsafe impl<T> Sync for SyncOnceCell<T> {}

    pub struct StaticCell<T: 'static> {
        inner: SyncOnceCell<T>,
    }

    impl<T> StaticCell<T> {
        #[doc(hidden)]
        pub const fn _from_once_cell(inner: OnceCell<T>) -> Self {
            Self {
                inner: SyncOnceCell(inner),
            }
        }

        pub fn get(&'static self) -> Option<&'static T> {
            self.inner.0.get()
        }

        pub fn set(&'static self, value: T) -> Result<(), T> {
            self.inner.0.set(value)
        }

        pub fn get_or_init<F>(&'static self, f: F) -> &'static T
        where
            F: FnOnce() -> T,
        {
            self.inner.0.get_or_init(f)
        }

        pub fn get_or_try_init<F, E>(&'static self, f: F) -> Result<&'static T, E>
        where
            F: FnOnce() -> Result<T, E>,
        {
            if let Some(val) = self.inner.0.get() {
                return Ok(val);
            }
            let val = f()?;
            let _ = self.inner.0.set(val);
            Ok(self.inner.0.get().unwrap())
        }
    }

    #[macro_export]
    macro_rules! static_cell {
        ($($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty;)+) => {
            $($(#[$attr])*
            $vis static $name: $crate::static_cell::StaticCell<$t> =
                $crate::static_cell::StaticCell::_from_once_cell($crate::lock::OnceCell::new());)+
        };
    }
}
#[cfg(all(not(feature = "threading"), not(feature = "std")))]
pub use no_std::*;
