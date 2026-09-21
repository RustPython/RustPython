// SSL exception types shared between ssl (rustls) and openssl backends

pub(crate) use ssl_error::*;

#[pymodule(sub)]
pub(crate) mod ssl_error {
    use crate::vm::{
        Py, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBaseException, PyOSError, PyStrRef},
        types::Constructor,
    };

    #[pyattr]
    pub(crate) use rustpython_host_env::ssl::{
        SSL_ERROR_EOF, SSL_ERROR_INVALID_ERROR_CODE, SSL_ERROR_NONE, SSL_ERROR_SSL,
        SSL_ERROR_SYSCALL, SSL_ERROR_WANT_CONNECT, SSL_ERROR_WANT_READ, SSL_ERROR_WANT_WRITE,
        SSL_ERROR_WANT_X509_LOOKUP, SSL_ERROR_ZERO_RETURN,
    };

    #[pyattr]
    #[pyexception(name = "SSLError", base = PyOSError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PySSLError(PyOSError);

    #[pyexception]
    impl PySSLError {
        // Returns strerror attribute if available, otherwise str(args)
        #[pymethod]
        fn __str__(zelf: &Py<PyBaseException>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            use crate::vm::AsObject;
            // Try to get strerror attribute first (OSError compatibility)
            if let Ok(strerror) = zelf.as_object().get_attr("strerror", vm)
                && !vm.is_none(&strerror)
            {
                return strerror.str(vm);
            }

            // Otherwise return str(args)
            let args = zelf.args();
            if args.len() == 1 {
                args.as_slice()[0].str(vm)
            } else {
                args.as_object().str(vm)
            }
        }
    }

    #[pyattr]
    #[pyexception(name = "SSLZeroReturnError", base = PySSLError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PySSLZeroReturnError(PySSLError);

    #[pyexception]
    impl PySSLZeroReturnError {}

    #[pyattr]
    #[pyexception(name = "SSLWantReadError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PySSLWantReadError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLWantWriteError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PySSLWantWriteError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLSyscallError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PySSLSyscallError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLEOFError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PySSLEOFError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLCertVerificationError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PySSLCertVerificationError(PySSLError);

    // Helper functions to create SSL exceptions with proper errno attribute
    #[cfg_attr(target_arch = "wasm32", expect(dead_code))]
    pub(crate) fn create_ssl_want_read_error(vm: &VirtualMachine) -> PyRef<PyOSError> {
        vm.new_os_subtype_error(
            PySSLWantReadError::class(&vm.ctx).to_owned(),
            Some(SSL_ERROR_WANT_READ),
            "The operation did not complete (read)",
        )
    }

    #[cfg_attr(target_arch = "wasm32", expect(dead_code))]
    pub(crate) fn create_ssl_want_write_error(vm: &VirtualMachine) -> PyRef<PyOSError> {
        vm.new_os_subtype_error(
            PySSLWantWriteError::class(&vm.ctx).to_owned(),
            Some(SSL_ERROR_WANT_WRITE),
            "The operation did not complete (write)",
        )
    }

    #[cfg_attr(target_arch = "wasm32", expect(dead_code))]
    pub(crate) fn create_ssl_eof_error(vm: &VirtualMachine) -> PyRef<PyOSError> {
        vm.new_os_subtype_error(
            PySSLEOFError::class(&vm.ctx).to_owned(),
            Some(SSL_ERROR_EOF),
            "EOF occurred in violation of protocol",
        )
    }

    #[cfg_attr(
        any(
            target_arch = "wasm32",
            all(feature = "ssl-openssl", not(feature = "ssl-rustls"))
        ),
        expect(dead_code)
    )]
    pub(crate) fn create_ssl_zero_return_error(vm: &VirtualMachine) -> PyRef<PyOSError> {
        vm.new_os_subtype_error(
            PySSLZeroReturnError::class(&vm.ctx).to_owned(),
            Some(SSL_ERROR_ZERO_RETURN),
            "TLS/SSL connection has been closed (EOF)",
        )
    }

    #[cfg_attr(
        any(
            target_arch = "wasm32",
            all(feature = "ssl-openssl", not(feature = "ssl-rustls"))
        ),
        expect(dead_code)
    )]
    pub(crate) fn create_ssl_syscall_error(
        vm: &VirtualMachine,
        msg: impl Into<String>,
    ) -> PyRef<PyOSError> {
        vm.new_os_subtype_error(
            PySSLSyscallError::class(&vm.ctx).to_owned(),
            Some(SSL_ERROR_SYSCALL),
            msg.into(),
        )
    }
}
