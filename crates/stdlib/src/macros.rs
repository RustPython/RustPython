#[macro_export]
macro_rules! flame_guard {
    ($name:expr) => {
        #[cfg(feature = "flame-it")]
        let _guard = ::flame::start_guard($name);
    };
}
