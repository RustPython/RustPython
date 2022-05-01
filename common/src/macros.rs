/// Suppress the MSVC invalid parameter handler, which by default crashes the process. Does nothing
/// on non-MSVC targets.
#[macro_export]
macro_rules! suppress_iph {
    ($e:expr) => {
        $crate::__suppress_iph_impl!($e)
    };
}

#[macro_export]
#[doc(hidden)]
#[cfg(all(windows, target_env = "msvc"))]
macro_rules! __suppress_iph_impl {
    ($e:expr) => {{
        let old = $crate::__macro_private::_set_thread_local_invalid_parameter_handler(
            $crate::__macro_private::silent_iph_handler,
        );
        let ret = $e;
        $crate::__macro_private::_set_thread_local_invalid_parameter_handler(old);
        ret
    }};
}

#[cfg(not(all(windows, target_env = "msvc")))]
#[macro_export]
#[doc(hidden)]
macro_rules! __suppress_iph_impl {
    ($e:expr) => {
        $e
    };
}

#[doc(hidden)]
pub mod __macro_private {
    #[cfg(target_env = "msvc")]
    type InvalidParamHandler = extern "C" fn(
        *const libc::wchar_t,
        *const libc::wchar_t,
        *const libc::wchar_t,
        libc::c_uint,
        libc::uintptr_t,
    );
    #[cfg(target_env = "msvc")]
    extern "C" {
        pub fn _set_thread_local_invalid_parameter_handler(
            pNew: InvalidParamHandler,
        ) -> InvalidParamHandler;
    }

    #[cfg(target_env = "msvc")]
    pub extern "C" fn silent_iph_handler(
        _: *const libc::wchar_t,
        _: *const libc::wchar_t,
        _: *const libc::wchar_t,
        _: libc::c_uint,
        _: libc::uintptr_t,
    ) {
    }
}
