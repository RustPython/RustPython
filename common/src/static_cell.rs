#[cfg(not(feature = "threading"))]
mod non_threading {
    use crate::lock::OnceCell;
    pub type StaticCell<T> = std::thread::LocalKey<OnceCell<&'static T>>;

    #[macro_export]
    macro_rules! static_cells {
        // process multiple declarations
        ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty;) => (
            std::thread_local! {
                $(#[$attr])* $vis static $name: $crate::lock::OnceCell<&'static $t> = $crate::lock::OnceCell::new();
            }
        );
    }
}
#[cfg(not(feature = "threading"))]
pub use non_threading::*;

#[cfg(feature = "threading")]
mod threading {
    use crate::lock::OnceCell;

    pub struct StaticKey<T> {
        inner: T,
    }

    impl<T> StaticKey<T> {
        pub const fn new(inner: T) -> Self {
            Self { inner }
        }

        pub fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&T) -> R,
        {
            f(&self.inner)
        }
    }

    pub type StaticCell<T> = StaticKey<OnceCell<&'static T>>;

    #[macro_export]
    macro_rules! static_cells {
        // process multiple declarations
        ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty;) => (
            $(#[$attr])* $vis static $name: $crate::static_cell::StaticKey<$crate::lock::OnceCell<&'static $t>> = $crate::static_cell::StaticKey::new($crate::lock::OnceCell::new());
        );
    }
}
#[cfg(feature = "threading")]
pub use threading::*;

pub fn get<T>(cell: &'static StaticCell<T>) -> Option<&'static T> {
    cell.with(|cell| cell.get().copied())
}

pub fn init_expect<T>(cell: &'static StaticCell<T>, value: T, msg: &'static str) -> &'static T {
    cell.with(|cell| {
        let static_ref = Box::leak(Box::new(value)) as &_;
        cell.set(static_ref)
            .unwrap_or_else(|_| panic!("double initializing '{}'", msg));
        static_ref
    })
}

pub fn get_or_init<T, F>(cell: &'static StaticCell<T>, f: F) -> &'static T
where
    F: FnOnce() -> T,
{
    cell.with(|cell| {
        *cell.get_or_init(|| {
            let value = f();
            Box::leak(Box::new(value))
        })
    })
}
