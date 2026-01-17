// SSL exception types shared between ssl (rustls) and openssl backends

pub(crate) use ssl_error::*;

#[pymodule(sub)]
pub(crate) mod ssl_error {
    use crate::vm::{
        Py, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBaseException, PyOSError, PyStrRef},
        types::Constructor,
    };

    // Error type constants - exposed as pyattr and available for internal use
    #[pyattr]
    pub(crate) const SSL_ERROR_NONE: i32 = 0;
    #[pyattr]
    pub(crate) const SSL_ERROR_SSL: i32 = 1;
    #[pyattr]
    pub(crate) const SSL_ERROR_WANT_READ: i32 = 2;
    #[pyattr]
    pub(crate) const SSL_ERROR_WANT_WRITE: i32 = 3;
    #[pyattr]
    pub(crate) const SSL_ERROR_WANT_X509_LOOKUP: i32 = 4;
    #[pyattr]
    pub(crate) const SSL_ERROR_SYSCALL: i32 = 5;
    #[pyattr]
    pub(crate) const SSL_ERROR_ZERO_RETURN: i32 = 6;
    #[pyattr]
    pub(crate) const SSL_ERROR_WANT_CONNECT: i32 = 7;
    #[pyattr]
    pub(crate) const SSL_ERROR_EOF: i32 = 8;
    #[pyattr]
    pub(crate) const SSL_ERROR_INVALID_ERROR_CODE: i32 = 10;

    #[pyattr]
    #[pyexception(name = "SSLError", base = PyOSError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PySSLError(PyOSError);

    #[pyexception]
    impl PySSLError {
        // Returns strerror attribute if available, otherwise str(args)
        #[pymethod]
        fn __str__(exc: &Py<PyBaseException>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            use crate::vm::AsObject;
            // Try to get strerror attribute first (OSError compatibility)
            if let Ok(strerror) = exc.as_object().get_attr("strerror", vm)
                && !vm.is_none(&strerror)
            {
                return strerror.str(vm);
            }

            // Otherwise return str(args)
            let args = exc.args();
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
    pub struct PySSLZeroReturnError(PySSLError);

    #[pyexception]
    impl PySSLZeroReturnError {}

    #[pyattr]
    #[pyexception(name = "SSLWantReadError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PySSLWantReadError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLWantWriteError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PySSLWantWriteError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLSyscallError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PySSLSyscallError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLEOFError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PySSLEOFError(PySSLError);

    #[pyattr]
    #[pyexception(name = "SSLCertVerificationError", base = PySSLError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PySSLCertVerificationError(PySSLError);

    // Helper functions to create SSL exceptions with proper errno attribute
    pub fn create_ssl_want_read_error(vm: &VirtualMachine) -> PyRef<PyOSError> {
        vm.new_os_subtype_error(
            PySSLWantReadError::class(&vm.ctx).to_owned(),
            Some(SSL_ERROR_WANT_READ),
            "The operation did not complete (read)",
        )
    }

    pub fn create_ssl_want_write_error(vm: &VirtualMachine) -> PyRef<PyOSError> {
        vm.new_os_subtype_error(
            PySSLWantWriteError::class(&vm.ctx).to_owned(),
            Some(SSL_ERROR_WANT_WRITE),
            "The operation did not complete (write)",
        )
    }

    pub fn create_ssl_eof_error(vm: &VirtualMachine) -> PyRef<PyOSError> {
        vm.new_os_subtype_error(
            PySSLEOFError::class(&vm.ctx).to_owned(),
            Some(SSL_ERROR_EOF),
            "EOF occurred in violation of protocol",
        )
    }

    pub fn create_ssl_zero_return_error(vm: &VirtualMachine) -> PyRef<PyOSError> {
        vm.new_os_subtype_error(
            PySSLZeroReturnError::class(&vm.ctx).to_owned(),
            Some(SSL_ERROR_ZERO_RETURN),
            "TLS/SSL connection has been closed (EOF)",
        )
    }
}
