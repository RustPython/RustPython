#[cfg(not(feature = "threading"))]
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
        pub const fn _from_localkey(inner: &'static LocalKey<OnceCell<&'static T>>) -> Self {
            Self { inner }
        }

        pub fn get(&'static self) -> Option<&'static T> {
            self.inner.with(|x| x.get().copied())
        }

        pub fn set(&'static self, value: T) -> Result<(), T> {
            // thread-safe because it's a unsync::OnceCell
            self.inner.with(|x| {
                if x.get().is_some() {
                    Err(value)
                } else {
                    // will never fail
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
            self.inner
                .with(|x| x.get_or_try_init(|| f().map(leak)).map(|&x| x))
        }
    }

    #[macro_export]
    macro_rules! static_cell {
        ($($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty;)+) => {
            $($(#[$attr])*
            $vis static $name: $crate::static_cell::StaticCell<$t> = {
                ::std::thread_local! {
                     $vis static $name: $crate::lock::OnceCell<&'static $t> = $crate::lock::OnceCell::new();
                }
                $crate::static_cell::StaticCell::_from_localkey(&$name)
            };)+
        };
    }
}
#[cfg(not(feature = "threading"))]
pub use non_threading::*;

#[cfg(feature = "threading")]
mod threading {
    use crate::lock::OnceCell;

    pub struct StaticCell<T: 'static> {
        inner: OnceCell<T>,
    }

    impl<T> StaticCell<T> {
        #[doc(hidden)]
        pub const fn _from_oncecell(inner: OnceCell<T>) -> Self {
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
            self.inner.get_or_try_init(f)
        }
    }

    #[macro_export]
    macro_rules! static_cell {
        ($($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty;)+) => {
            $($(#[$attr])*
            $vis static $name: $crate::static_cell::StaticCell<$t> =
                $crate::static_cell::StaticCell::_from_oncecell($crate::lock::OnceCell::new());)+
        };
    }
}
#[cfg(feature = "threading")]
pub use threading::*;
