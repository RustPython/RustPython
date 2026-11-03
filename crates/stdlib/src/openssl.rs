// spell-checker:disable

mod cert;
#[cfg(all(unix, not(target_os = "aix")))]
mod driver;
#[cfg(all(unix, not(target_os = "aix")))]
mod waiter;

// SSL exception types (shared with rustls backend)
#[allow(
    clippy::duplicate_mod,
    reason = "This is duplicated only when running clippy. The two features are mutually exclusive"
)]
#[path = "ssl/error.rs"]
mod ssl_error;

// Conditional compilation for OpenSSL version-specific error codes
cfg_select! {
    // OpenSSL 3.1.0+
    ossl310 => {
        mod ssl_data_31;
        use ssl_data_31 as ssl_data;
    }
    // OpenSSL 3.0.0+
    ossl300 => {
        mod ssl_data_300;
        use ssl_data_300 as ssl_data;
    }
    // OpenSSL 1.1.1+ (fallback)
    _ => {
        mod ssl_data_111;
        use ssl_data_111 as ssl_data;
    }
}

pub(crate) use _ssl::module_def;

#[allow(non_upper_case_globals)]
#[pymodule(with(
    cert::ssl_cert,
    ssl_error::ssl_error,
    #[cfg(ossl101)] ossl101,
    #[cfg(ossl111)] ossl111,
    #[cfg(windows)] windows))]
mod _ssl {
    use core::hint::cold_path;

    use super::bio;
    #[cfg(all(unix, not(target_os = "aix")))]
    use super::{
        driver::{Driver, Needs, Permit, WriteOwner},
        waiter::{ProgressRegistry, ProgressWaiter},
    };
    #[cfg(not(all(unix, not(target_os = "aix"))))]
    use crate::socket::{SockWaitKind, sock_wait};
    #[cfg(all(unix, not(target_os = "aix")))]
    use alloc::sync::Arc;
    #[cfg(all(unix, not(target_os = "aix")))]
    use std::os::fd::AsRawFd;

    // Import error types and helpers used in this module (others are exposed via pymodule(with(...)))
    use super::ssl_error::{
        PySSLCertVerificationError, PySSLError, create_ssl_eof_error, create_ssl_want_read_error,
        create_ssl_want_write_error,
    };
    use crate::{
        common::lock::{
            LazyLock, PyDetachingRwLock, PyMappedRwLockReadGuard, PyMutex, PyRwLock,
            PyRwLockReadGuard, PyRwLockWriteGuard,
        },
        socket::{self, PySocket},
        vm::{
            AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
            builtins::{
                PyBaseException, PyBaseExceptionRef, PyBytesRef, PyListRef, PyModule, PyStrRef,
                PyType, PyUtf8StrRef, PyWeak,
            },
            class_or_notimplemented,
            convert::ToPyException,
            exceptions,
            function::{
                ArgBytesLike, ArgMemoryBuffer, ArgStrOrBytesLike, Either, FsPath, OptionalArg,
                PyComparisonValue,
            },
            object::{Traverse, TraverseFn},
            stdlib::_warnings,
            types::{Comparable, Constructor, PyComparisonOp},
            utils::ToCString,
        },
    };
    use crossbeam_utils::atomic::AtomicCell;
    use foreign_types_shared::{ForeignType, ForeignTypeRef};
    use openssl::{
        asn1::{Asn1Object, Asn1ObjectRef},
        error::ErrorStack,
        nid::Nid,
        ssl::{self, SslContextBuilder, SslOptions, SslVerifyMode},
        x509::X509,
    };
    use openssl_sys as sys;
    use rustpython_vm::ospath::OsPath;

    use core::{ffi::CStr, fmt};
    use std::{
        io::{Read, Write},
        path::PathBuf,
        time::Instant,
    };

    // Import certificate types from parent module
    use super::cert::{self, cert_to_certificate, cert_to_py};

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        Ok(())
    }

    // Re-export PySSLCertificate to make it available in the _ssl module
    // It will be automatically exposed to Python via #[pyclass]
    #[allow(unused_imports)]
    use super::cert::PySSLCertificate;

    // Constants
    #[pyattr]
    use sys::{
        // SSL Alert Descriptions that are exported by openssl_sys
        SSL_AD_DECODE_ERROR,
        SSL_AD_ILLEGAL_PARAMETER,
        SSL_AD_UNRECOGNIZED_NAME,
        // SSL_ERROR_INVALID_ERROR_CODE,
        SSL_ERROR_SSL,
        // SSL_ERROR_WANT_X509_LOOKUP,
        SSL_ERROR_SYSCALL,
        SSL_ERROR_WANT_CONNECT,
        SSL_ERROR_WANT_READ,
        SSL_ERROR_WANT_WRITE,
        SSL_ERROR_ZERO_RETURN,
        SSL_OP_CIPHER_SERVER_PREFERENCE as OP_CIPHER_SERVER_PREFERENCE,
        SSL_OP_ENABLE_MIDDLEBOX_COMPAT as OP_ENABLE_MIDDLEBOX_COMPAT,
        SSL_OP_LEGACY_SERVER_CONNECT as OP_LEGACY_SERVER_CONNECT,
        SSL_OP_NO_SSLv2 as OP_NO_SSLv2,
        SSL_OP_NO_SSLv3 as OP_NO_SSLv3,
        SSL_OP_NO_TICKET as OP_NO_TICKET,
        SSL_OP_NO_TLSv1 as OP_NO_TLSv1,
        SSL_OP_SINGLE_DH_USE as OP_SINGLE_DH_USE,
        SSL_OP_SINGLE_ECDH_USE as OP_SINGLE_ECDH_USE,
        X509_V_FLAG_ALLOW_PROXY_CERTS as VERIFY_ALLOW_PROXY_CERTS,
        X509_V_FLAG_CRL_CHECK as VERIFY_CRL_CHECK_LEAF,
        X509_V_FLAG_PARTIAL_CHAIN as VERIFY_X509_PARTIAL_CHAIN,
        X509_V_FLAG_TRUSTED_FIRST as VERIFY_X509_TRUSTED_FIRST,
        X509_V_FLAG_X509_STRICT as VERIFY_X509_STRICT,
    };

    #[pyattr]
    use rustpython_host_env::ssl::{
        ALERT_DESCRIPTION_ACCESS_DENIED, ALERT_DESCRIPTION_BAD_CERTIFICATE,
        ALERT_DESCRIPTION_BAD_CERTIFICATE_HASH_VALUE,
        ALERT_DESCRIPTION_BAD_CERTIFICATE_STATUS_RESPONSE, ALERT_DESCRIPTION_BAD_RECORD_MAC,
        ALERT_DESCRIPTION_CERTIFICATE_EXPIRED, ALERT_DESCRIPTION_CERTIFICATE_REQUIRED,
        ALERT_DESCRIPTION_CERTIFICATE_REVOKED, ALERT_DESCRIPTION_CERTIFICATE_UNKNOWN,
        ALERT_DESCRIPTION_CERTIFICATE_UNOBTAINABLE, ALERT_DESCRIPTION_CLOSE_NOTIFY,
        ALERT_DESCRIPTION_DECODE_ERROR, ALERT_DESCRIPTION_DECOMPRESSION_FAILURE,
        ALERT_DESCRIPTION_DECRYPT_ERROR, ALERT_DESCRIPTION_DECRYPTION_FAILED,
        ALERT_DESCRIPTION_EXPORT_RESTRICTION, ALERT_DESCRIPTION_HANDSHAKE_FAILURE,
        ALERT_DESCRIPTION_ILLEGAL_PARAMETER, ALERT_DESCRIPTION_INAPPROPRIATE_FALLBACK,
        ALERT_DESCRIPTION_INSUFFICIENT_SECURITY, ALERT_DESCRIPTION_INTERNAL_ERROR,
        ALERT_DESCRIPTION_MISSING_EXTENSION, ALERT_DESCRIPTION_NO_APPLICATION_PROTOCOL,
        ALERT_DESCRIPTION_NO_CERTIFICATE, ALERT_DESCRIPTION_NO_RENEGOTIATION,
        ALERT_DESCRIPTION_PROTOCOL_VERSION, ALERT_DESCRIPTION_RECORD_OVERFLOW,
        ALERT_DESCRIPTION_UNEXPECTED_MESSAGE, ALERT_DESCRIPTION_UNKNOWN_CA,
        ALERT_DESCRIPTION_UNKNOWN_PSK_IDENTITY, ALERT_DESCRIPTION_UNRECOGNIZED_NAME,
        ALERT_DESCRIPTION_UNSUPPORTED_CERTIFICATE, ALERT_DESCRIPTION_UNSUPPORTED_EXTENSION,
        ALERT_DESCRIPTION_USER_CANCELLED,
    };

    // CRL verification constants
    #[pyattr]
    const VERIFY_CRL_CHECK_CHAIN: core::ffi::c_ulong =
        sys::X509_V_FLAG_CRL_CHECK | sys::X509_V_FLAG_CRL_CHECK_ALL;

    // taken from CPython, should probably be kept up to date with their version if it ever changes
    #[pyattr]
    const _DEFAULT_CIPHERS: &str =
        "DEFAULT:!aNULL:!eNULL:!MD5:!3DES:!DES:!RC4:!IDEA:!SEED:!aDSS:!SRP:!PSK";
    // #[pyattr] PROTOCOL_SSLv2: u32 = SslVersion::Ssl2 as u32;  // unsupported
    // #[pyattr] PROTOCOL_SSLv3: u32 = SslVersion::Ssl3 as u32;
    #[pyattr]
    const PROTOCOL_SSLv23: u32 = SslVersion::Tls as u32;
    #[pyattr]
    const PROTOCOL_TLS: u32 = SslVersion::Tls as u32;
    #[pyattr]
    const PROTOCOL_TLS_CLIENT: u32 = SslVersion::TlsClient as u32;
    #[pyattr]
    const PROTOCOL_TLS_SERVER: u32 = SslVersion::TlsServer as u32;
    #[pyattr]
    const PROTOCOL_TLSv1: u32 = SslVersion::Tls1 as u32;
    #[pyattr]
    const PROTOCOL_TLSv1_1: u32 = SslVersion::Tls1_1 as u32;
    #[pyattr]
    const PROTOCOL_TLSv1_2: u32 = SslVersion::Tls1_2 as u32;
    #[pyattr]
    const PROTO_MINIMUM_SUPPORTED: i32 = ProtoVersion::MinSupported as i32;
    #[pyattr]
    const PROTO_SSLv3: i32 = ProtoVersion::Ssl3 as i32;
    #[pyattr]
    const PROTO_TLSv1: i32 = ProtoVersion::Tls1 as i32;
    #[pyattr]
    const PROTO_TLSv1_1: i32 = ProtoVersion::Tls1_1 as i32;
    #[pyattr]
    const PROTO_TLSv1_2: i32 = ProtoVersion::Tls1_2 as i32;
    #[pyattr]
    const PROTO_TLSv1_3: i32 = ProtoVersion::Tls1_3 as i32;
    #[pyattr]
    const PROTO_MAXIMUM_SUPPORTED: i32 = ProtoVersion::MaxSupported as i32;
    #[pyattr]
    const OP_ALL: core::ffi::c_ulong =
        (sys::SSL_OP_ALL & !sys::SSL_OP_DONT_INSERT_EMPTY_FRAGMENTS) as _;
    #[pyattr]
    const HAS_TLS_UNIQUE: bool = true;
    #[pyattr]
    const CERT_NONE: u32 = CertRequirements::None as u32;
    #[pyattr]
    const CERT_OPTIONAL: u32 = CertRequirements::Optional as u32;
    #[pyattr]
    const CERT_REQUIRED: u32 = CertRequirements::Required as u32;
    #[pyattr]
    const VERIFY_DEFAULT: u32 = 0;
    #[pyattr]
    const HAS_SNI: bool = true;
    #[pyattr]
    const HAS_ECDH: bool = true;
    #[pyattr]
    const HAS_NPN: bool = false;
    #[pyattr]
    const HAS_ALPN: bool = true;
    #[pyattr]
    const HAS_SSLv2: bool = false;
    #[pyattr]
    const HAS_SSLv3: bool = false;
    #[pyattr]
    const HAS_TLSv1: bool = true;
    #[pyattr]
    const HAS_TLSv1_1: bool = true;
    #[pyattr]
    const HAS_TLSv1_2: bool = true;
    #[pyattr]
    const HAS_TLSv1_3: bool = cfg!(ossl111);
    #[pyattr]
    const HAS_PSK: bool = false; // External PSK callbacks are not implemented.
    #[pyattr]
    const HAS_PSK_TLS13: bool = false;
    #[pyattr]
    const HAS_PHA: bool = cfg!(ossl111);

    // Encoding constants for Certificate.public_bytes()
    #[pyattr]
    pub(crate) const ENCODING_PEM: i32 = sys::X509_FILETYPE_PEM;
    #[pyattr]
    pub(crate) const ENCODING_DER: i32 = sys::X509_FILETYPE_ASN1;
    #[pyattr]
    const ENCODING_PEM_AUX: i32 = sys::X509_FILETYPE_PEM + 0x100;

    // OpenSSL error codes for unexpected EOF detection
    const ERR_LIB_SSL: i32 = 20;
    const SSL_R_UNEXPECTED_EOF_WHILE_READING: i32 = 294;

    // SSL_VERIFY constants for post-handshake authentication
    #[cfg(ossl111)]
    use sys::SSL_VERIFY_POST_HANDSHAKE;

    // the openssl version from the API headers

    #[pyattr(name = "OPENSSL_VERSION")]
    fn openssl_version(_vm: &VirtualMachine) -> &str {
        openssl::version::version()
    }
    #[pyattr(name = "OPENSSL_VERSION_NUMBER")]
    fn openssl_version_number(_vm: &VirtualMachine) -> i64 {
        openssl::version::number()
    }
    #[pyattr(name = "OPENSSL_VERSION_INFO")]
    fn openssl_version_info(_vm: &VirtualMachine) -> OpensslVersionInfo {
        parse_version_info(openssl::version::number())
    }

    #[pyattr(name = "_OPENSSL_API_VERSION")]
    fn _openssl_api_version(_vm: &VirtualMachine) -> OpensslVersionInfo {
        let openssl_api_version = i64::from_str_radix(env!("OPENSSL_API_VERSION"), 16)
            .expect("OPENSSL_API_VERSION is malformed");
        parse_version_info(openssl_api_version)
    }

    type OpensslVersionInfo = (u8, u8, u8, u8, u8);
    const fn parse_version_info(mut n: i64) -> OpensslVersionInfo {
        let status = (n & 0xF) as u8;
        n >>= 4;
        let patch = (n & 0xFF) as u8;
        n >>= 8;
        let fix = (n & 0xFF) as u8;
        n >>= 8;
        let minor = (n & 0xFF) as u8;
        n >>= 8;
        let major = (n & 0xFF) as u8;
        (major, minor, fix, patch, status)
    }

    #[derive(Copy, Clone, num_enum::IntoPrimitive, num_enum::TryFromPrimitive, PartialEq)]
    #[repr(i32)]
    enum SslVersion {
        Ssl2,
        Ssl3 = 1,
        Tls,
        Tls1,
        Tls1_1,
        Tls1_2,
        TlsClient = 0x10,
        TlsServer,
    }

    #[derive(Copy, Clone, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
    #[repr(i32)]
    enum ProtoVersion {
        MinSupported = -2,
        Ssl3 = sys::SSL3_VERSION,
        Tls1 = sys::TLS1_VERSION,
        Tls1_1 = sys::TLS1_1_VERSION,
        Tls1_2 = sys::TLS1_2_VERSION,
        #[cfg(ossl111)]
        Tls1_3 = sys::TLS1_3_VERSION,
        #[cfg(not(ossl111))]
        Tls1_3 = 0x304,
        MaxSupported = -1,
    }

    #[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
    #[repr(i32)]
    enum CertRequirements {
        None,
        Optional,
        Required,
    }

    #[derive(Debug, PartialEq)]
    enum SslServerOrClient {
        Client,
        Server,
    }

    unsafe fn ptr2obj(ptr: *mut sys::ASN1_OBJECT) -> Option<Asn1Object> {
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { Asn1Object::from_ptr(ptr) })
        }
    }

    fn _txt2obj(s: &CStr, no_name: bool) -> Option<Asn1Object> {
        unsafe { ptr2obj(sys::OBJ_txt2obj(s.as_ptr(), i32::from(no_name))) }
    }
    fn _nid2obj(nid: Nid) -> Option<Asn1Object> {
        unsafe { ptr2obj(sys::OBJ_nid2obj(nid.as_raw())) }
    }

    type PyNid = (core::ffi::c_int, String, String, Option<String>);
    fn obj2py(obj: &Asn1ObjectRef, vm: &VirtualMachine) -> PyResult<PyNid> {
        let nid = obj.nid();
        let short_name = nid
            .short_name()
            .map_err(|_| vm.new_value_error("NID has no short name"))?
            .to_owned();
        let long_name = nid
            .long_name()
            .map_err(|_| vm.new_value_error("NID has no long name"))?
            .to_owned();
        Ok((
            nid.as_raw(),
            short_name,
            long_name,
            cert::obj2txt(obj, true),
        ))
    }

    #[derive(FromArgs)]
    struct Txt2ObjArgs {
        txt: PyStrRef,
        #[pyarg(any, default)]
        name: bool,
    }

    #[pyfunction]
    fn txt2obj(args: Txt2ObjArgs, vm: &VirtualMachine) -> PyResult<PyNid> {
        _txt2obj(&args.txt.to_cstring(vm)?, !args.name)
            .as_deref()
            .ok_or_else(|| vm.new_value_error(format!("unknown object '{}'", args.txt)))
            .and_then(|obj| obj2py(obj, vm))
    }

    #[pyfunction]
    fn nid2obj(nid: core::ffi::c_int, vm: &VirtualMachine) -> PyResult<PyNid> {
        _nid2obj(Nid::from_raw(nid))
            .as_deref()
            .ok_or_else(|| vm.new_value_error(format!("unknown NID {nid}")))
            .and_then(|obj| obj2py(obj, vm))
    }

    fn path_from_cstr(c: &CStr) -> PathBuf {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            std::ffi::OsStr::from_bytes(c.to_bytes()).into()
        }
        #[cfg(windows)]
        {
            PathBuf::from(c.to_string_lossy().as_ref())
        }
    }

    fn openssl_default_cert_paths() -> (PathBuf, PathBuf) {
        (
            path_from_cstr(unsafe { CStr::from_ptr(sys::X509_get_default_cert_file()) }),
            path_from_cstr(unsafe { CStr::from_ptr(sys::X509_get_default_cert_dir()) }),
        )
    }

    fn get_cert_file_dir() -> (PathBuf, PathBuf) {
        cfg_select! {
            all(openssl_vendored, not(target_arch = "wasm32")) => {
                let (cert_file, cert_dir) =
                    rustpython_host_env::native_certs::default_verify_paths();
                (PathBuf::from(cert_file), PathBuf::from(cert_dir))
            }
            _ => openssl_default_cert_paths(),
        }
    }

    // Lazily compute and cache cert environment variable names
    static CERT_ENV_NAMES: LazyLock<(String, String)> = LazyLock::new(|| {
        let cert_file_env = unsafe { CStr::from_ptr(sys::X509_get_default_cert_file_env()) }
            .to_string_lossy()
            .into_owned();
        let cert_dir_env = unsafe { CStr::from_ptr(sys::X509_get_default_cert_dir_env()) }
            .to_string_lossy()
            .into_owned();
        (cert_file_env, cert_dir_env)
    });

    #[pyfunction]
    fn get_default_verify_paths(
        vm: &VirtualMachine,
    ) -> PyResult<(&'static str, PyObjectRef, &'static str, PyObjectRef)> {
        let (cert_file_env, cert_dir_env) = &*CERT_ENV_NAMES;
        let (cert_file, cert_dir) = get_cert_file_dir();
        let cert_file = OsPath::new_str(cert_file.as_path()).filename(vm);
        let cert_dir = OsPath::new_str(cert_dir.as_path()).filename(vm);
        Ok((
            cert_file_env.as_str(),
            cert_file,
            cert_dir_env.as_str(),
            cert_dir,
        ))
    }

    #[pyfunction(name = "RAND_status")]
    fn rand_status() -> i32 {
        unsafe { sys::RAND_status() }
    }

    #[pyfunction(name = "RAND_add")]
    fn rand_add(string: ArgStrOrBytesLike, entropy: f64) {
        let f = |b: &[u8]| {
            for buf in b.chunks(core::ffi::c_int::MAX as usize) {
                unsafe { sys::RAND_add(buf.as_ptr() as *const _, buf.len() as _, entropy) }
            }
        };
        f(&string.borrow_bytes())
    }

    #[pyfunction(name = "RAND_bytes")]
    fn rand_bytes(n: i32, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        if n < 0 {
            return Err(vm.new_value_error("num must be positive"));
        }
        let mut buf = vm.new_zeroed_bytes(n as usize)?;
        openssl::rand::rand_bytes(&mut buf).map_err(|e| convert_openssl_error(vm, e))?;
        Ok(buf)
    }

    type ActiveSslConnection = (*const PyDetachingRwLock<SslConnection>, *mut sys::SSL);

    // Thread-local storage for VirtualMachine pointer during handshake
    // SNI callback is only called during handshake which is synchronous
    thread_local! {
        static HANDSHAKE_VM: core::cell::Cell<Option<*const VirtualMachine>> = const { core::cell::Cell::new(None) };
        // SSL pointer during handshake - needed because connection lock is held during handshake
        // and callbacks may need to access SSL without acquiring the lock
        static HANDSHAKE_SSL_PTR: core::cell::Cell<Option<*mut sys::SSL>> = const { core::cell::Cell::new(None) };
        #[cfg(all(unix, not(target_os = "aix")))]
        static SSL_IO_CALLS: core::cell::RefCell<Vec<*const PySslSocket>> = const { core::cell::RefCell::new(Vec::new()) };
        static HANDSHAKE_CONNECTIONS: core::cell::RefCell<Vec<ActiveSslConnection>> = const { core::cell::RefCell::new(Vec::new()) };
    }

    // RAII guard to set/clear thread-local handshake context
    struct HandshakeVmGuard {
        previous_vm: Option<*const VirtualMachine>,
        previous_ssl: Option<*mut sys::SSL>,
    }

    impl HandshakeVmGuard {
        fn new(
            vm: &VirtualMachine,
            connection: &PyDetachingRwLock<SslConnection>,
            ssl_ptr: *mut sys::SSL,
        ) -> Self {
            HANDSHAKE_CONNECTIONS.with(|connections| {
                connections
                    .borrow_mut()
                    .push((connection as *const _, ssl_ptr));
            });
            Self {
                previous_vm: HANDSHAKE_VM.with(|cell| cell.replace(Some(vm as *const _))),
                previous_ssl: HANDSHAKE_SSL_PTR.with(|cell| cell.replace(Some(ssl_ptr))),
            }
        }
    }

    impl Drop for HandshakeVmGuard {
        fn drop(&mut self) {
            HANDSHAKE_VM.with(|cell| cell.set(self.previous_vm));
            HANDSHAKE_SSL_PTR.with(|cell| cell.set(self.previous_ssl));
            HANDSHAKE_CONNECTIONS.with(|connections| {
                connections.borrow_mut().pop();
            });
        }
    }

    fn active_ssl_pointer(connection: &PyDetachingRwLock<SslConnection>) -> Option<*mut sys::SSL> {
        HANDSHAKE_CONNECTIONS.with(|connections| {
            connections
                .borrow()
                .iter()
                .rev()
                .find_map(|&(owner, pointer)| core::ptr::eq(owner, connection).then_some(pointer))
        })
    }

    fn with_ssl_pointer<T>(
        connection: &PyDetachingRwLock<SslConnection>,
        operation: impl FnOnce(*mut sys::SSL) -> T,
    ) -> T {
        if let Some(pointer) = active_ssl_pointer(connection) {
            // This thread already holds this connection's write guard.
            return operation(pointer);
        }
        let guard = connection.read();
        operation(guard.ssl().as_ptr())
    }

    fn with_ssl_pointer_mut<T>(
        connection: &PyDetachingRwLock<SslConnection>,
        operation: impl FnOnce(*mut sys::SSL) -> T,
    ) -> T {
        if let Some(pointer) = active_ssl_pointer(connection) {
            return operation(pointer);
        }
        let guard = connection.write();
        operation(guard.ssl().as_ptr())
    }

    // The owning _SSLSocket outlives its SSL connection. Borrow its stable
    // address, as CPython's SSL app_data does, without creating a hidden cycle.
    fn get_socket_ex_data_index() -> core::ffi::c_int {
        static SOCKET_EX_DATA_IDX: LazyLock<core::ffi::c_int> = LazyLock::new(|| unsafe {
            sys::SSL_get_ex_new_index(0, core::ptr::null_mut(), None, None, None)
        });
        *SOCKET_EX_DATA_IDX
    }

    // SNI callback function called by OpenSSL
    unsafe extern "C" fn _servername_callback(
        ssl_ptr: *mut sys::SSL,
        al: *mut core::ffi::c_int,
        _arg: *mut core::ffi::c_void,
    ) -> core::ffi::c_int {
        const SSL_TLSEXT_ERR_OK: core::ffi::c_int = 0;
        const SSL_TLSEXT_ERR_ALERT_FATAL: core::ffi::c_int = 2;
        const SSL_AD_INTERNAL_ERROR: core::ffi::c_int = 80;
        const SSL_AD_HANDSHAKE_FAILURE: core::ffi::c_int = 40;
        const TLSEXT_NAMETYPE_host_name: core::ffi::c_int = 0;

        unsafe {
            let socket_ptr = sys::SSL_get_ex_data(ssl_ptr, get_socket_ex_data_index());
            if socket_ptr.is_null() {
                *al = SSL_AD_INTERNAL_ERROR;
                return SSL_TLSEXT_ERR_ALERT_FATAL;
            }
            let ssl_socket = &*(socket_ptr as *const Py<PySslSocket>);

            // Get VM from thread-local storage (set by HandshakeVmGuard in do_handshake)
            let Some(vm_ptr) = HANDSHAKE_VM.with(|cell| cell.get()) else {
                // VM not available - this shouldn't happen during handshake
                *al = SSL_AD_INTERNAL_ERROR;
                return SSL_TLSEXT_ERR_ALERT_FATAL;
            };
            let vm = &*vm_ptr;

            // The handshake this runs inside has left the interpreter, so
            // everything below rejoins it first — taking a reference to the
            // callback already counts — and gives the thread back after.
            vm.attach_for_callback(|| {
                // The callback may replace the socket's context and release its
                // last outside reference. Keep both context and callback alive.
                let ctx = ssl_socket.ctx.read().clone();
                let callback_opt = ctx.sni_callback.lock().clone();
                let Some(callback) = callback_opt else {
                    return SSL_TLSEXT_ERR_OK;
                };

                // Get server name
                let servername = sys::SSL_get_servername(ssl_ptr, TLSEXT_NAMETYPE_host_name);
                let server_name_arg = if servername.is_null() {
                    vm.ctx.none()
                } else {
                    let name_cstr = core::ffi::CStr::from_ptr(servername);
                    match name_cstr.to_str() {
                        Ok(name_str) => vm.ctx.new_str(name_str).into(),
                        Err(_) => vm.ctx.none(),
                    }
                };

                let ssl_socket_obj = match PySslSocket::callback_owner(ssl_socket) {
                    Some(owner) => owner,
                    None => {
                        *al = SSL_AD_INTERNAL_ERROR;
                        return SSL_TLSEXT_ERR_ALERT_FATAL;
                    }
                };

                match callback.call((ssl_socket_obj, server_name_arg, ctx), vm) {
                    Ok(result) => {
                        // Check return value type (must be None or integer)
                        if vm.is_none(&result) {
                            // None is OK
                            SSL_TLSEXT_ERR_OK
                        } else {
                            // Try to convert to integer
                            match result.try_to_value::<i32>(vm) {
                                Ok(alert_code) => {
                                    // Valid integer - use as alert code
                                    *al = alert_code;
                                    SSL_TLSEXT_ERR_ALERT_FATAL
                                }
                                Err(_) => {
                                    // Type conversion failed - raise TypeError
                                    let type_error = vm.new_type_error(format!(
                                    "servername callback must return None or an integer, not '{}'",
                                    result.class().name()
                                ));
                                    vm.run_unraisable(type_error, None, result);
                                    *al = SSL_AD_INTERNAL_ERROR;
                                    SSL_TLSEXT_ERR_ALERT_FATAL
                                }
                            }
                        }
                    }
                    Err(exc) => {
                        // Log the exception but don't propagate it
                        vm.run_unraisable(exc, None, vm.ctx.none());
                        *al = SSL_AD_HANDSHAKE_FAILURE;
                        SSL_TLSEXT_ERR_ALERT_FATAL
                    }
                }
            })
        }
    }

    // OpenSSL record type constants for msg_callback
    const SSL3_RT_CHANGE_CIPHER_SPEC: i32 = 20;
    const SSL3_RT_ALERT: i32 = 21;
    const SSL3_RT_HANDSHAKE: i32 = 22;
    const SSL3_RT_HEADER: i32 = 256;
    const SSL3_RT_INNER_CONTENT_TYPE: i32 = 257;
    // Special value for change cipher spec (CPython compatibility)
    const SSL3_MT_CHANGE_CIPHER_SPEC: i32 = 0x0101;

    // Message callback function called by OpenSSL
    // Called during SSL operations to report protocol messages.
    // debughelpers.c:_PySSL_msg_callback
    unsafe extern "C" fn _msg_callback(
        write_p: core::ffi::c_int,
        mut version: core::ffi::c_int,
        content_type: core::ffi::c_int,
        buf: *const core::ffi::c_void,
        len: usize,
        ssl_ptr: *mut sys::SSL,
        _arg: *mut core::ffi::c_void,
    ) {
        if ssl_ptr.is_null() {
            return;
        }

        unsafe {
            // Get SSL socket from ex_data using the dedicated index
            let idx = get_socket_ex_data_index();
            let ssl_socket_ptr = sys::SSL_get_ex_data(ssl_ptr, idx);
            if ssl_socket_ptr.is_null() {
                return;
            }

            // Borrowed stable address, installed by _wrap_socket/_wrap_bio.
            let ssl_socket: &Py<PySslSocket> = &*(ssl_socket_ptr as *const Py<PySslSocket>);

            // Get VM from thread-local storage (set by HandshakeVmGuard in do_handshake)
            let Some(vm_ptr) = HANDSHAKE_VM.with(|cell| cell.get()) else {
                // VM not available - this shouldn't happen during handshake
                return;
            };
            let vm = &*vm_ptr;

            // The SSL call this reports from has left the interpreter; rejoin
            // it for the duration of the callback, as `_servername_callback`
            // does above.
            vm.attach_for_callback(|| {
                // Get the callback from the context
                let callback_opt = ssl_socket.ctx.read().msg_callback.lock().clone();
                let Some(callback) = callback_opt else {
                    return;
                };

                let Some(ssl_socket_obj) = PySslSocket::callback_owner(ssl_socket) else {
                    return;
                };

                // Create the message bytes
                let buf_slice = core::slice::from_raw_parts(buf as *const u8, len);
                let msg_bytes = vm.ctx.new_bytes(buf_slice.to_vec());

                // Determine direction string
                let direction_str = if write_p != 0 { "write" } else { "read" };

                // Calculate msg_type based on content_type (debughelpers.c behavior)
                let msg_type = match content_type {
                    SSL3_RT_CHANGE_CIPHER_SPEC => SSL3_MT_CHANGE_CIPHER_SPEC,
                    SSL3_RT_ALERT if len >= 2 => {
                        // byte 1 is alert type
                        buf_slice[1] as i32
                    }
                    SSL3_RT_HANDSHAKE if !buf_slice.is_empty() => {
                        // byte 0 is handshake type
                        buf_slice[0] as i32
                    }
                    SSL3_RT_HEADER if len >= 3 => {
                        // Frame header: version in bytes 1..2, type in byte 0
                        version = ((buf_slice[1] as i32) << 8) | (buf_slice[2] as i32);
                        buf_slice[0] as i32
                    }
                    SSL3_RT_INNER_CONTENT_TYPE if !buf_slice.is_empty() => {
                        // Inner content type in byte 0
                        buf_slice[0] as i32
                    }
                    _ => -1,
                };

                // Call the Python callback
                // Signature: callback(conn, direction, version, content_type, msg_type, data)
                match callback.call(
                    (
                        ssl_socket_obj,
                        vm.ctx.new_str(direction_str),
                        vm.ctx.new_int(version),
                        vm.ctx.new_int(content_type),
                        vm.ctx.new_int(msg_type),
                        msg_bytes,
                    ),
                    vm,
                ) {
                    Ok(_) => {}
                    Err(exc) => {
                        // SSL message callbacks cannot signal failure to the C
                        // library. Propagate it when the enclosing operation
                        // rejoins the interpreter, chaining earlier callbacks.
                        let previous = ssl_socket.callback_error.lock().replace(exc.clone());
                        if let Some(previous) = previous {
                            exc.set_context(Some(previous));
                        }
                    }
                }
            })
        }
    }

    #[pyfunction(name = "RAND_pseudo_bytes")]
    fn rand_pseudo_bytes(n: i32, vm: &VirtualMachine) -> PyResult<(Vec<u8>, bool)> {
        if n < 0 {
            return Err(vm.new_value_error("num must be positive"));
        }
        let mut buf = vm.new_zeroed_bytes(n as usize)?;
        let ret = unsafe { sys::RAND_bytes(buf.as_mut_ptr(), n) };
        match ret {
            0 | 1 => Ok((buf, ret == 1)),
            _ => Err(convert_openssl_error(vm, ErrorStack::get())),
        }
    }

    #[pyattr]
    #[pyclass(module = "_ssl", name = "_SSLContext", traverse)]
    #[derive(PyPayload)]
    struct PySslContext {
        #[pytraverse(skip)]
        ctx: PyRwLock<SslContextBuilder>,
        #[pytraverse(skip)]
        check_hostname: AtomicCell<bool>,
        #[pytraverse(skip)]
        protocol: SslVersion,
        #[pytraverse(skip)]
        post_handshake_auth: PyMutex<bool>,
        sni_callback: PyMutex<Option<PyObjectRef>>,
        msg_callback: PyMutex<Option<PyObjectRef>>,
    }

    impl fmt::Debug for PySslContext {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("_SSLContext")
        }
    }

    impl Drop for PySslContext {
        fn drop(&mut self) {
            // SSL retains its original session context across context switches.
            // It must not call back through a released Python context on HRR.
            unsafe {
                sys::SSL_CTX_set_tlsext_servername_callback__fixed_rust(
                    self.ctx.get_mut().as_ptr(),
                    None,
                );
            }
        }
    }

    fn builder_as_ctx(x: &SslContextBuilder) -> &ssl::SslContextRef {
        unsafe { ssl::SslContextRef::from_ptr(x.as_ptr()) }
    }

    impl Constructor for PySslContext {
        type Args = i32;

        fn py_new(
            _cls: &Py<PyType>,
            proto_version: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let proto = SslVersion::try_from(proto_version)
                .map_err(|_| vm.new_value_error("invalid protocol version"))?;
            let (method, deprecated_protocol) = match proto {
                // SslVersion::Ssl3 => unsafe { ssl::SslMethod::from_ptr(sys::SSLv3_method()) },
                SslVersion::Tls => (ssl::SslMethod::tls(), Some("PROTOCOL_TLS")),
                SslVersion::Tls1 => (ssl::SslMethod::tls(), Some("PROTOCOL_TLSv1")),
                SslVersion::Tls1_1 => (ssl::SslMethod::tls(), Some("PROTOCOL_TLSv1_1")),
                SslVersion::Tls1_2 => (ssl::SslMethod::tls(), Some("PROTOCOL_TLSv1_2")),
                SslVersion::TlsClient => (ssl::SslMethod::tls_client(), None),
                SslVersion::TlsServer => (ssl::SslMethod::tls_server(), None),
                _ => return Err(vm.new_value_error("invalid protocol version")),
            };
            if let Some(protocol_name) = deprecated_protocol {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!("ssl.{protocol_name} is deprecated"),
                    2,
                    vm,
                )?;
            }
            let mut builder =
                SslContextBuilder::new(method).map_err(|e| convert_openssl_error(vm, e))?;

            #[cfg(target_os = "android")]
            android::load_client_ca_list(vm, &mut builder)?;

            let check_hostname = proto == SslVersion::TlsClient;
            builder.set_verify(if check_hostname {
                SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT
            } else {
                SslVerifyMode::NONE
            });

            // Start with OP_ALL but remove options that CPython doesn't include by default
            let mut options = SslOptions::ALL & !SslOptions::DONT_INSERT_EMPTY_FRAGMENTS;
            if proto != SslVersion::Ssl2 {
                options |= SslOptions::NO_SSLV2;
            }
            if proto != SslVersion::Ssl3 {
                options |= SslOptions::NO_SSLV3;
            }
            options |= SslOptions::NO_COMPRESSION;
            options |= SslOptions::CIPHER_SERVER_PREFERENCE;
            options |= SslOptions::SINGLE_DH_USE;
            options |= SslOptions::SINGLE_ECDH_USE;
            options |= SslOptions::ENABLE_MIDDLEBOX_COMPAT;
            builder.set_options(options);
            // Remove NO_TLSv1 and NO_TLSv1_1 which newer OpenSSL adds to OP_ALL
            builder.clear_options(SslOptions::NO_TLSV1 | SslOptions::NO_TLSV1_1);

            let mode = ssl::SslMode::ACCEPT_MOVING_WRITE_BUFFER | ssl::SslMode::AUTO_RETRY;
            builder.set_mode(mode);

            #[cfg(ossl111)]
            unsafe {
                sys::SSL_CTX_set_post_handshake_auth(builder.as_ptr(), 0);
            }

            // Note: Unlike some other implementations, we do NOT set session_id_context at the
            // context level. CPython sets it only on individual SSL objects (server-side only).
            // This matches CPython's behavior in _ssl.c where SSL_set_session_id_context is called
            // in newPySSLSocket() at line 862, not during context creation.

            // Set protocol version limits based on the protocol version
            unsafe {
                let ctx_ptr = builder.as_ptr();
                match proto {
                    SslVersion::Tls1 => {
                        sys::SSL_CTX_set_min_proto_version(ctx_ptr, sys::TLS1_VERSION);
                        sys::SSL_CTX_set_max_proto_version(ctx_ptr, sys::TLS1_VERSION);
                    }
                    SslVersion::Tls1_1 => {
                        sys::SSL_CTX_set_min_proto_version(ctx_ptr, sys::TLS1_1_VERSION);
                        sys::SSL_CTX_set_max_proto_version(ctx_ptr, sys::TLS1_1_VERSION);
                    }
                    SslVersion::Tls1_2 => {
                        sys::SSL_CTX_set_min_proto_version(ctx_ptr, sys::TLS1_2_VERSION);
                        sys::SSL_CTX_set_max_proto_version(ctx_ptr, sys::TLS1_2_VERSION);
                    }
                    _ => {
                        // For Tls, TlsClient, TlsServer, use default (no restrictions)
                    }
                }
            }

            // Set default verify flags: VERIFY_X509_TRUSTED_FIRST
            unsafe {
                let ctx_ptr = builder.as_ptr();
                let param = sys::SSL_CTX_get0_param(ctx_ptr);
                sys::X509_VERIFY_PARAM_set_flags(param, sys::X509_V_FLAG_TRUSTED_FIRST);
            }

            Ok(PySslContext {
                ctx: PyRwLock::new(builder),
                check_hostname: AtomicCell::new(check_hostname),
                protocol: proto,
                post_handshake_auth: PyMutex::new(false),
                sni_callback: PyMutex::new(None),
                msg_callback: PyMutex::new(None),
            })
        }
    }

    #[pyclass(flags(BASETYPE, IMMUTABLETYPE), with(Constructor))]
    impl PySslContext {
        fn warn_deprecated_tls_version(version: i32, vm: &VirtualMachine) -> PyResult<()> {
            let version_name = match version {
                PROTO_SSLv3 => Some("SSLv3"),
                PROTO_TLSv1 => Some("TLSv1"),
                PROTO_TLSv1_1 => Some("TLSv1_1"),
                _ => None,
            };
            if let Some(version_name) = version_name {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!("ssl.TLSVersion.{version_name} is deprecated"),
                    2,
                    vm,
                )?;
            }
            Ok(())
        }

        fn builder(&self) -> PyRwLockWriteGuard<'_, SslContextBuilder> {
            self.ctx.write()
        }
        fn ctx(&self) -> PyMappedRwLockReadGuard<'_, ssl::SslContextRef> {
            PyRwLockReadGuard::map(self.ctx.read(), builder_as_ctx)
        }

        #[pygetset]
        fn post_handshake_auth(zelf: &Py<Self>) -> bool {
            *zelf.post_handshake_auth.lock()
        }
        #[pygetset(setter)]
        fn set_post_handshake_auth(
            zelf: &Py<Self>,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let value = value.ok_or_else(|| vm.new_attribute_error("cannot delete attribute"))?;
            *zelf.post_handshake_auth.lock() = value.is_true(vm)?;
            Ok(())
        }

        #[cfg(ossl110)]
        #[pygetset]
        fn security_level(zelf: &Py<Self>) -> i32 {
            unsafe { SSL_CTX_get_security_level(zelf.ctx().as_ptr()) }
        }

        #[pymethod]
        fn set_ciphers(zelf: &Py<Self>, cipherlist: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
            if cipherlist.contains_nuls() {
                cold_path();
                return Err(exceptions::nul_char_error(vm));
            }

            zelf.builder()
                .set_cipher_list(cipherlist.as_ref())
                .map_err(|_| new_ssl_error(vm, "No cipher can be selected."))
        }

        #[pymethod]
        fn set_ciphersuites(
            zelf: &Py<Self>,
            ciphersuites: PyUtf8StrRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let list = ciphersuites.as_str().to_cstring(vm)?;
            #[cfg(ossl111)]
            {
                let result = unsafe {
                    sys::SSL_CTX_set_ciphersuites(zelf.builder().as_ptr(), list.as_ptr())
                };
                if result != 1 {
                    let _ = ErrorStack::get();
                    return Err(new_ssl_error(vm, "No cipher suite can be selected."));
                }
                Ok(())
            }
            #[cfg(not(ossl111))]
            {
                let _ = (zelf, list);
                Err(vm.new_not_implemented_error("TLS 1.3 requires OpenSSL 1.1.1 or later."))
            }
        }

        fn set_algorithm_list(
            &self,
            list: PyUtf8StrRef,
            command: core::ffi::c_int,
            message: &str,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let list = list.as_str().to_cstring(vm)?;
            let result = unsafe {
                sys::SSL_CTX_ctrl(
                    self.builder().as_ptr(),
                    command,
                    0,
                    list.as_ptr().cast_mut().cast(),
                )
            };
            if result != 1 {
                let _ = ErrorStack::get();
                return Err(new_ssl_error(vm, message));
            }
            Ok(())
        }

        #[pymethod]
        fn set_groups(
            zelf: &Py<Self>,
            grouplist: PyUtf8StrRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // SSL_CTX_set1_groups_list is a macro around SSL_CTX_ctrl.
            const SSL_CTRL_SET_GROUPS_LIST: core::ffi::c_int = 92;
            zelf.set_algorithm_list(
                grouplist,
                SSL_CTRL_SET_GROUPS_LIST,
                "unrecognized group",
                vm,
            )
        }

        #[pymethod]
        fn set_server_sigalgs(
            zelf: &Py<Self>,
            sigalgslist: PyUtf8StrRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            const SSL_CTRL_SET_SIGALGS_LIST: core::ffi::c_int = 98;
            zelf.set_algorithm_list(
                sigalgslist,
                SSL_CTRL_SET_SIGALGS_LIST,
                "unrecognized signature algorithm",
                vm,
            )
        }

        #[pymethod]
        fn set_client_sigalgs(
            zelf: &Py<Self>,
            sigalgslist: PyUtf8StrRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            const SSL_CTRL_SET_CLIENT_SIGALGS_LIST: core::ffi::c_int = 102;
            zelf.set_algorithm_list(
                sigalgslist,
                SSL_CTRL_SET_CLIENT_SIGALGS_LIST,
                "unrecognized signature algorithm",
                vm,
            )
        }

        #[pymethod]
        fn get_groups(
            zelf: &Py<Self>,
            args: GetGroupsArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyListRef> {
            #[cfg(ossl350)]
            unsafe {
                const SSL_CTRL_GET0_IMPLEMENTED_GROUPS: core::ffi::c_int = 139;
                let groups = sys::OPENSSL_sk_new_null();
                if groups.is_null() {
                    return Err(vm.new_memory_error("failed to allocate group list"));
                }
                let result = sys::SSL_CTX_ctrl(
                    zelf.ctx().as_ptr(),
                    SSL_CTRL_GET0_IMPLEMENTED_GROUPS,
                    core::ffi::c_long::from(args.include_aliases),
                    groups.cast(),
                );
                if result == 0 {
                    sys::OPENSSL_sk_free(groups);
                    return Err(convert_openssl_error(vm, ErrorStack::get()));
                }
                // The stack owns its storage, but OpenSSL owns each name.
                let names = (0..sys::OPENSSL_sk_num(groups))
                    .map(|index| {
                        CStr::from_ptr(sys::OPENSSL_sk_value(groups, index).cast())
                            .to_string_lossy()
                            .into_owned()
                    })
                    .collect::<Vec<_>>();
                sys::OPENSSL_sk_free(groups);
                Ok(vm.ctx.new_list(
                    names
                        .into_iter()
                        .map(|name| vm.ctx.new_str(name).into())
                        .collect(),
                ))
            }
            #[cfg(not(ossl350))]
            {
                let _ = (zelf, args.include_aliases);
                Err(vm.new_not_implemented_error("Getting groups requires OpenSSL 3.5 or later."))
            }
        }

        #[pymethod]
        fn get_ciphers(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyListRef> {
            let ctx = zelf.ctx();
            let ssl = ssl::Ssl::new(&ctx).map_err(|e| convert_openssl_error(vm, e))?;

            unsafe {
                let ciphers_ptr = SSL_get_ciphers(ssl.as_ptr());
                if ciphers_ptr.is_null() {
                    return Ok(vm.ctx.new_list(vec![]));
                }

                let num_ciphers = sys::OPENSSL_sk_num(ciphers_ptr as *const _);
                let mut result = Vec::new();

                for i in 0..num_ciphers {
                    let cipher_ptr =
                        sys::OPENSSL_sk_value(ciphers_ptr as *const _, i) as *const sys::SSL_CIPHER;
                    let cipher = ssl::SslCipherRef::from_ptr(cipher_ptr as *mut _);

                    let (name, version, bits) = cipher_to_tuple(cipher);
                    let dict = vm.ctx.new_dict();
                    dict.set_item("name", vm.ctx.new_str(name).into(), vm)?;
                    dict.set_item("protocol", vm.ctx.new_str(version).into(), vm)?;
                    dict.set_item("secret_bits", vm.ctx.new_int(bits).into(), vm)?;

                    // Add description field
                    let description = cipher_description(cipher_ptr);
                    dict.set_item("description", vm.ctx.new_str(description).into(), vm)?;

                    result.push(dict.into());
                }

                Ok(vm.ctx.new_list(result))
            }
        }

        #[pymethod]
        fn set_ecdh_curve(
            zelf: &Py<Self>,
            name: Either<PyStrRef, ArgBytesLike>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            use openssl::ec::{EcGroup, EcKey};

            // Convert name to CString, supporting both str and bytes
            let name_cstr = match name {
                Either::A(s) => {
                    let s: &str = s.as_ref();
                    s.to_cstring(vm)?
                }
                Either::B(b) => std::ffi::CString::new(b.borrow_buf().to_vec())
                    .map_err(|_| exceptions::nul_char_error(vm))?,
            };

            // Find the NID for the curve name using OBJ_sn2nid
            let nid_raw = unsafe { sys::OBJ_sn2nid(name_cstr.as_ptr()) };
            if nid_raw == 0 {
                return Err(vm.new_value_error("unknown curve name"));
            }
            let nid = Nid::from_raw(nid_raw);

            // Create EC key from the curve
            let group = EcGroup::from_curve_name(nid).map_err(|e| convert_openssl_error(vm, e))?;
            let key = EcKey::from_group(&group).map_err(|e| convert_openssl_error(vm, e))?;

            // Set the temporary ECDH key
            zelf.builder()
                .set_tmp_ecdh(&key)
                .map_err(|e| convert_openssl_error(vm, e))
        }

        #[pygetset]
        fn options(zelf: &Py<Self>) -> core::ffi::c_ulong {
            zelf.ctx.read().options().bits() as _
        }
        #[pygetset(setter)]
        fn set_options(zelf: &Py<Self>, new_opts: i64, vm: &VirtualMachine) -> PyResult<()> {
            if new_opts < 0 {
                return Err(vm.new_value_error("invalid options value"));
            }
            let new_opts = new_opts as core::ffi::c_ulong;
            let current = {
                let ctx = zelf.ctx();
                unsafe { sys::SSL_CTX_get_options(ctx.as_ptr()) }
            };

            // Calculate options to clear and set
            let clear = current & !new_opts;
            let set = !current & new_opts;

            let opt_no = sys::SSL_OP_NO_SSLv2
                | sys::SSL_OP_NO_SSLv3
                | sys::SSL_OP_NO_TLSv1
                | sys::SSL_OP_NO_TLSv1_1
                | sys::SSL_OP_NO_TLSv1_2;
            #[cfg(ossl111)]
            let opt_no = opt_no | sys::SSL_OP_NO_TLSv1_3;
            if (set & opt_no) != 0 {
                _warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    "ssl.OP_NO_SSL*/ssl.OP_NO_TLS* options are deprecated".to_owned(),
                    2,
                    vm,
                )?;
            }

            let mut ctx = zelf.builder();
            // Clear options first (using raw FFI since openssl crate doesn't expose clear_options)
            if clear != 0 {
                unsafe {
                    sys::SSL_CTX_clear_options(ctx.as_ptr(), clear);
                }
            }

            // Then set new options
            if set != 0 {
                ctx.set_options(SslOptions::from_bits_truncate(set as _));
            }
            Ok(())
        }
        #[pygetset]
        fn protocol(zelf: &Py<Self>) -> i32 {
            zelf.protocol as i32
        }
        #[pygetset]
        fn verify_mode(zelf: &Py<Self>) -> i32 {
            let mode = zelf.ctx().verify_mode();
            if mode == SslVerifyMode::NONE {
                CertRequirements::None.into()
            } else if mode == SslVerifyMode::PEER {
                CertRequirements::Optional.into()
            } else if mode == SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT {
                CertRequirements::Required.into()
            } else {
                unreachable!()
            }
        }
        #[pygetset(setter)]
        fn set_verify_mode(zelf: &Py<Self>, cert: i32, vm: &VirtualMachine) -> PyResult<()> {
            let mut ctx = zelf.builder();
            let cert_req = CertRequirements::try_from(cert)
                .map_err(|_| vm.new_value_error("invalid value for verify_mode"))?;
            let mode = match cert_req {
                CertRequirements::None if zelf.check_hostname.load() => {
                    return Err(vm.new_value_error(
                        "Cannot set verify_mode to CERT_NONE when check_hostname is enabled.",
                    ));
                }
                CertRequirements::None => SslVerifyMode::NONE,
                CertRequirements::Optional => SslVerifyMode::PEER,
                CertRequirements::Required => {
                    SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT
                }
            };
            ctx.set_verify(mode);
            Ok(())
        }
        #[pygetset]
        fn verify_flags(zelf: &Py<Self>) -> core::ffi::c_ulong {
            unsafe {
                let ctx_ptr = zelf.ctx().as_ptr();
                let param = sys::SSL_CTX_get0_param(ctx_ptr);
                sys::X509_VERIFY_PARAM_get_flags(param)
            }
        }
        #[pygetset(setter)]
        fn set_verify_flags(
            zelf: &Py<Self>,
            new_flags: core::ffi::c_ulong,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            unsafe {
                let ctx_ptr = zelf.ctx().as_ptr();
                let param = sys::SSL_CTX_get0_param(ctx_ptr);
                let flags = sys::X509_VERIFY_PARAM_get_flags(param);
                let clear = flags & !new_flags;
                let set = !flags & new_flags;

                if clear != 0 && sys::X509_VERIFY_PARAM_clear_flags(param, clear) == 0 {
                    return Err(new_ssl_error(vm, "Failed to clear verify flags"));
                }
                if set != 0 && sys::X509_VERIFY_PARAM_set_flags(param, set) == 0 {
                    return Err(new_ssl_error(vm, "Failed to set verify flags"));
                }
                Ok(())
            }
        }
        #[pygetset]
        fn check_hostname(zelf: &Py<Self>) -> bool {
            zelf.check_hostname.load()
        }
        #[pygetset(setter)]
        fn set_check_hostname(zelf: &Py<Self>, ch: bool) {
            let mut ctx = zelf.builder();
            if ch && builder_as_ctx(&ctx).verify_mode() == SslVerifyMode::NONE {
                ctx.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
            }
            zelf.check_hostname.store(ch);
        }

        // PY_PROTO_MINIMUM_SUPPORTED = -2, PY_PROTO_MAXIMUM_SUPPORTED = -1
        #[pygetset]
        fn minimum_version(zelf: &Py<Self>) -> i32 {
            if !matches!(
                zelf.protocol,
                SslVersion::Tls | SslVersion::TlsClient | SslVersion::TlsServer
            ) {
                return -2;
            }
            let ctx = zelf.ctx();
            let version = unsafe { sys::SSL_CTX_get_min_proto_version(ctx.as_ptr()) };
            if version == 0 {
                -2 // PY_PROTO_MINIMUM_SUPPORTED
            } else {
                version
            }
        }
        #[pygetset(setter)]
        fn set_minimum_version(zelf: &Py<Self>, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            if !matches!(
                zelf.protocol,
                SslVersion::Tls | SslVersion::TlsClient | SslVersion::TlsServer
            ) {
                return Err(vm.new_value_error(
                    "The context's protocol doesn't support modification of highest and lowest version.",
                ));
            }
            ProtoVersion::try_from(value)
                .map_err(|_| vm.new_value_error("invalid protocol version"))?;
            Self::warn_deprecated_tls_version(value, vm)?;

            // Handle special values
            let proto_version = match value {
                -2 => {
                    // Zero removes the explicit minimum protocol limit.
                    0
                }
                -1 => {
                    // PY_PROTO_MAXIMUM_SUPPORTED -> use maximum available
                    // For max on min_proto_version, we use the newest available
                    sys::TLS1_3_VERSION
                }
                _ => value,
            };

            let ctx = zelf.builder();
            let result = unsafe { sys::SSL_CTX_set_min_proto_version(ctx.as_ptr(), proto_version) };
            if result == 0 {
                return Err(vm.new_value_error("invalid protocol version"));
            }
            Ok(())
        }

        #[pygetset]
        fn maximum_version(zelf: &Py<Self>) -> i32 {
            if !matches!(
                zelf.protocol,
                SslVersion::Tls | SslVersion::TlsClient | SslVersion::TlsServer
            ) {
                return -1;
            }
            let ctx = zelf.ctx();
            let version = unsafe { sys::SSL_CTX_get_max_proto_version(ctx.as_ptr()) };
            if version == 0 {
                -1 // PY_PROTO_MAXIMUM_SUPPORTED
            } else {
                version
            }
        }
        #[pygetset(setter)]
        fn set_maximum_version(zelf: &Py<Self>, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            if !matches!(
                zelf.protocol,
                SslVersion::Tls | SslVersion::TlsClient | SslVersion::TlsServer
            ) {
                return Err(vm.new_value_error(
                    "The context's protocol doesn't support modification of highest and lowest version.",
                ));
            }
            ProtoVersion::try_from(value)
                .map_err(|_| vm.new_value_error("invalid protocol version"))?;
            Self::warn_deprecated_tls_version(value, vm)?;

            // Handle special values
            let proto_version = match value {
                -1 => {
                    // PY_PROTO_MAXIMUM_SUPPORTED -> use 0 for OpenSSL (means no limit)
                    0
                }
                // This backend advertises TLSv1 as its lowest supported version.
                -2 => sys::TLS1_VERSION,
                _ => value,
            };

            let ctx = zelf.builder();
            let result = unsafe { sys::SSL_CTX_set_max_proto_version(ctx.as_ptr(), proto_version) };
            if result == 0 {
                return Err(vm.new_value_error("invalid protocol version"));
            }
            Ok(())
        }

        #[pygetset]
        fn num_tickets(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<usize> {
            // Only supported for TLS 1.3
            cfg_select! {
                ossl111 => {
                    let ctx = zelf.ctx();
                    let num = unsafe { sys::SSL_CTX_get_num_tickets(ctx.as_ptr()) };
                    Ok(num)
                }
                _ => Ok(0),
            }
        }
        #[pygetset(setter)]
        fn set_num_tickets(zelf: &Py<Self>, value: isize, vm: &VirtualMachine) -> PyResult<()> {
            // Check for negative values
            if value < 0 {
                return Err(vm.new_value_error("num_tickets must be a non-negative integer"));
            }

            // Check that this is a server context
            if zelf.protocol != SslVersion::TlsServer {
                return Err(vm.new_value_error("SSLContext is not a server context."));
            }

            cfg_select! {
                ossl110 => {
                    let ctx = zelf.builder();
                    let result =
                        unsafe { sys::SSL_CTX_set_num_tickets(ctx.as_ptr(), value as usize) };
                    if result != 1 {
                        return Err(vm.new_value_error("failed to set num tickets."));
                    }
                    Ok(())
                }
                _ => {
                    let _ = (value, vm);
                    Ok(())
                }
            }
        }

        #[pymethod]
        fn set_default_verify_paths(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            cfg_select! {
                openssl_vendored => {
                    let (cert_file, cert_dir) = get_cert_file_dir();
                    zelf.builder()
                        .load_verify_locations(Some(cert_file.as_path()), Some(cert_dir.as_path()))
                        .map_err(|e| convert_openssl_error(vm, e))
                }
                _ => zelf
                    .builder()
                    .set_default_verify_paths()
                    .map_err(|e| convert_openssl_error(vm, e)),
            }
        }

        #[pymethod]
        fn _set_alpn_protocols(
            zelf: &Py<Self>,
            protos: ArgBytesLike,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            #[cfg(ossl102)]
            {
                let mut ctx = zelf.builder();
                let server = protos.with_ref(|pbuf| {
                    if pbuf.len() > core::ffi::c_uint::MAX as usize {
                        return Err(vm.new_overflow_error(format!(
                            "protocols longer than {} bytes",
                            core::ffi::c_uint::MAX
                        )));
                    }
                    ctx.set_alpn_protos(pbuf)
                        .map_err(|e| convert_openssl_error(vm, e))?;
                    Ok(pbuf.to_vec())
                })?;
                ctx.set_alpn_select_callback(move |_, client| {
                    let proto =
                        ssl::select_next_proto(&server, client).ok_or(ssl::AlpnError::NOACK)?;
                    let pos = memchr::memmem::find(client, proto)
                        .expect("selected alpn proto should be present in client protos");
                    Ok(&client[pos..pos + proto.len()])
                });
                Ok(())
            }
            #[cfg(not(ossl102))]
            {
                Err(vm.new_not_implemented_error(
                    "The NPN extension requires OpenSSL 1.0.1 or later.",
                ))
            }
        }

        #[pymethod]
        fn set_psk_client_callback(
            _zelf: &Py<Self>,
            _callback: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Err(vm.new_not_implemented_error(
                "TLS-PSK callbacks are not implemented by this backend.",
            ))
        }

        #[pymethod]
        fn set_psk_server_callback(
            _zelf: &Py<Self>,
            _callback: PyObjectRef,
            _identity_hint: OptionalArg<PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Err(vm.new_not_implemented_error(
                "TLS-PSK callbacks are not implemented by this backend.",
            ))
        }

        #[pymethod]
        fn load_verify_locations(
            zelf: &Py<Self>,
            args: LoadVerifyLocationsArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if let (None, None, None) = (&args.cafile, &args.capath, &args.cadata) {
                return Err(vm.new_type_error("cafile, capath and cadata cannot be all omitted"));
            }

            #[cold]
            fn invalid_cadata(vm: &VirtualMachine) -> PyBaseExceptionRef {
                vm.new_type_error("cadata should be an ASCII string or a bytes-like object")
            }

            let mut ctx = zelf.builder();

            // validate cadata type and load cadata
            if let Some(cadata) = args.cadata {
                let (certs, is_pem) = match cadata {
                    Either::A(s) => {
                        let s: &str = s.as_ref();
                        if !s.is_ascii() {
                            return Err(invalid_cadata(vm));
                        }
                        (X509::stack_from_pem(s.as_bytes()), true)
                    }
                    Either::B(b) => (b.with_ref(x509_stack_from_der), false),
                };
                let certs = certs.map_err(|e| convert_openssl_error(vm, e))?;

                // If no certificates were loaded, raise an error
                if certs.is_empty() {
                    let msg = if is_pem {
                        "no start line: cadata does not contain a certificate"
                    } else {
                        "not enough data: cadata does not contain a certificate"
                    };
                    return Err(vm
                        .new_os_subtype_error(
                            PySSLError::class(&vm.ctx).to_owned(),
                            None,
                            msg.to_owned(),
                        )
                        .upcast());
                }

                let store = ctx.cert_store_mut();
                for cert in certs {
                    store
                        .add_cert(cert)
                        .map_err(|e| convert_openssl_error(vm, e))?;
                }
            }

            if args.cafile.is_some() || args.capath.is_some() {
                let cafile_path = args.cafile.map(|p| p.to_path_buf(vm)).transpose()?;
                let capath_path = args.capath.map(|p| p.to_path_buf(vm)).transpose()?;
                // Check file/directory existence before calling OpenSSL to get proper errno
                if let Some(ref path) = cafile_path
                    && !rustpython_host_env::fs::exists(path)
                {
                    return Err(vm
                        .new_os_subtype_error(
                            vm.ctx.exceptions.file_not_found_error.to_owned(),
                            Some(rustpython_host_env::errno::errors::ENOENT),
                            format!("No such file or directory: '{}'", path.display()),
                        )
                        .upcast());
                }
                if let Some(ref path) = capath_path
                    && !rustpython_host_env::fs::exists(path)
                {
                    return Err(vm
                        .new_os_subtype_error(
                            vm.ctx.exceptions.file_not_found_error.to_owned(),
                            Some(rustpython_host_env::errno::errors::ENOENT),
                            format!("No such file or directory: '{}'", path.display()),
                        )
                        .upcast());
                }
                ctx.load_verify_locations(cafile_path.as_deref(), capath_path.as_deref())
                    .map_err(|e| convert_openssl_error(vm, e))?;
            }

            Ok(())
        }

        #[pymethod]
        fn get_ca_certs(
            zelf: &Py<Self>,
            args: GetCertArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<PyObjectRef>> {
            let binary_form = args.binary_form.unwrap_or(false);
            let ctx = zelf.ctx();
            let certs = cfg_select! {
                ossl300 => ctx.cert_store().all_certificates(),
                _ => ctx.cert_store().objects().iter().filter_map(|x| x.x509()),
            };

            // Filter to only include CA certificates (Basic Constraints: CA=TRUE)
            let certs = certs
                .into_iter()
                .filter(|cert| {
                    unsafe {
                        // X509_check_ca() returns 1 for CA certificates
                        X509_check_ca(cert.as_ptr()) == 1
                    }
                })
                .map(|ref cert| cert_to_py(vm, cert, binary_form))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(certs)
        }

        #[pymethod]
        fn cert_store_stats(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let ctx = zelf.ctx();
            let store_ptr = unsafe { sys::SSL_CTX_get_cert_store(ctx.as_ptr()) };

            if store_ptr.is_null() {
                return Err(vm.new_memory_error("failed to get cert store"));
            }

            let objs_ptr = unsafe { sys::X509_STORE_get0_objects(store_ptr) };
            if objs_ptr.is_null() {
                return Err(vm.new_memory_error("failed to query cert store"));
            }

            let mut x509_count = 0;
            let mut crl_count = 0;
            let mut ca_count = 0;

            unsafe {
                let num_objs = sys::OPENSSL_sk_num(objs_ptr as *const _);
                for i in 0..num_objs {
                    let obj_ptr =
                        sys::OPENSSL_sk_value(objs_ptr as *const _, i) as *const sys::X509_OBJECT;
                    let obj_type = X509_OBJECT_get_type(obj_ptr);

                    match obj_type {
                        X509_LU_X509 => {
                            x509_count += 1;
                            let x509_ptr = sys::X509_OBJECT_get0_X509(obj_ptr);
                            // X509_check_ca returns non-zero for any CA type
                            if !x509_ptr.is_null() && X509_check_ca(x509_ptr) != 0 {
                                ca_count += 1;
                            }
                        }
                        X509_LU_CRL => {
                            crl_count += 1;
                        }
                        _ => {
                            // Ignore unrecognized types
                        }
                    }
                }
                // Note: No need to free objs_ptr as X509_STORE_get0_objects returns
                // a pointer to internal data that should not be freed by the caller
            }

            let dict = vm.ctx.new_dict();
            dict.set_item("x509", vm.ctx.new_int(x509_count).into(), vm)?;
            dict.set_item("crl", vm.ctx.new_int(crl_count).into(), vm)?;
            dict.set_item("x509_ca", vm.ctx.new_int(ca_count).into(), vm)?;
            Ok(dict.into())
        }

        #[pymethod]
        fn session_stats(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let ctx = zelf.ctx();
            let ctx_ptr = ctx.as_ptr();

            let dict = vm.ctx.new_dict();

            macro_rules! add_stat {
                ($key:expr, $func:ident) => {
                    let value = unsafe { $func(ctx_ptr) };
                    dict.set_item($key, vm.ctx.new_int(value).into(), vm)?;
                };
            }

            add_stat!("number", SSL_CTX_sess_number);
            add_stat!("connect", SSL_CTX_sess_connect);
            add_stat!("connect_good", SSL_CTX_sess_connect_good);
            add_stat!("connect_renegotiate", SSL_CTX_sess_connect_renegotiate);
            add_stat!("accept", SSL_CTX_sess_accept);
            add_stat!("accept_good", SSL_CTX_sess_accept_good);
            add_stat!("accept_renegotiate", SSL_CTX_sess_accept_renegotiate);
            add_stat!("hits", SSL_CTX_sess_hits);
            add_stat!("misses", SSL_CTX_sess_misses);
            add_stat!("timeouts", SSL_CTX_sess_timeouts);
            add_stat!("cache_full", SSL_CTX_sess_cache_full);

            Ok(dict.into())
        }

        #[pymethod]
        fn load_dh_params(zelf: &Py<Self>, filepath: FsPath, vm: &VirtualMachine) -> PyResult<()> {
            let path = filepath.to_path_buf(vm)?;

            // Open the file using fopen (cross-platform)
            let fp =
                rustpython_host_env::fileutils::fopen(path.as_path(), "rb").map_err(|e| match e
                    .kind()
                {
                    std::io::ErrorKind::NotFound => vm
                        .new_os_subtype_error(
                            vm.ctx.exceptions.file_not_found_error.to_owned(),
                            Some(rustpython_host_env::errno::errors::ENOENT),
                            e.to_string(),
                        )
                        .upcast(),
                    _ => vm.new_os_error(e.to_string()),
                })?;

            // Read DH parameters
            let dh = unsafe {
                PEM_read_DHparams(
                    fp,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                )
            };
            unsafe {
                rustpython_host_env::fileutils::fclose(fp);
            }

            if dh.is_null() {
                return Err(convert_openssl_error(vm, ErrorStack::get()));
            }

            // Set temporary DH parameters
            let ctx = zelf.builder();
            let result = unsafe { sys::SSL_CTX_set_tmp_dh(ctx.as_ptr(), dh) };
            unsafe {
                sys::DH_free(dh);
            }

            if result != 1 {
                return Err(convert_openssl_error(vm, ErrorStack::get()));
            }

            Ok(())
        }

        #[pygetset]
        fn sni_callback(zelf: &Py<Self>) -> Option<PyObjectRef> {
            zelf.sni_callback.lock().clone()
        }

        #[pygetset(setter)]
        fn set_sni_callback(
            zelf: &Py<Self>,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // Check if this is a server context
            if zelf.protocol == SslVersion::TlsClient {
                return Err(vm.new_value_error("sni_callback cannot be set on TLS_CLIENT context"));
            }

            let mut callback_guard = zelf.sni_callback.lock();

            if let Some(callback_obj) = value {
                if !vm.is_none(&callback_obj) {
                    // Check if callable
                    if !callback_obj.is_callable() {
                        return Err(vm.new_type_error("not a callable object"));
                    }

                    // Set the callback
                    *callback_guard = Some(callback_obj);

                    // Set OpenSSL callback
                    unsafe {
                        sys::SSL_CTX_set_tlsext_servername_callback__fixed_rust(
                            zelf.ctx().as_ptr(),
                            Some(_servername_callback),
                        );
                    }
                } else {
                    // Clear callback
                    *callback_guard = None;
                    unsafe {
                        sys::SSL_CTX_set_tlsext_servername_callback__fixed_rust(
                            zelf.ctx().as_ptr(),
                            None,
                        );
                    }
                }
            } else {
                // Clear callback
                *callback_guard = None;
                unsafe {
                    sys::SSL_CTX_set_tlsext_servername_callback__fixed_rust(
                        zelf.ctx().as_ptr(),
                        None,
                    );
                }
            }

            Ok(())
        }

        #[pymethod]
        fn set_servername_callback(
            zelf: &Py<Self>,
            callback: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Self::set_sni_callback(zelf, callback, vm)
        }

        #[pygetset(name = "_msg_callback")]
        fn msg_callback(zelf: &Py<Self>) -> Option<PyObjectRef> {
            zelf.msg_callback.lock().clone()
        }

        #[pygetset(setter, name = "_msg_callback")]
        fn set_msg_callback(
            zelf: &Py<Self>,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let mut callback_guard = zelf.msg_callback.lock();

            if let Some(callback_obj) = value {
                if !vm.is_none(&callback_obj) {
                    // Check if callable
                    if !callback_obj.is_callable() {
                        return Err(vm.new_type_error("not a callable object"));
                    }

                    // Set the callback
                    *callback_guard = Some(callback_obj);

                    // Set OpenSSL callback
                    unsafe {
                        SSL_CTX_set_msg_callback(zelf.ctx().as_ptr(), Some(_msg_callback));
                    }
                } else {
                    // Clear callback
                    *callback_guard = None;
                    unsafe {
                        SSL_CTX_set_msg_callback(zelf.ctx().as_ptr(), None);
                    }
                }
            } else {
                // Clear callback when value is None
                *callback_guard = None;
                unsafe {
                    SSL_CTX_set_msg_callback(zelf.ctx().as_ptr(), None);
                }
            }

            Ok(())
        }

        #[pymethod]
        fn load_cert_chain(
            zelf: &Py<Self>,
            args: LoadCertChainArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            use core::cell::RefCell;
            use openssl::pkey::PKey;

            let LoadCertChainArgs {
                certfile,
                keyfile,
                password,
            } = args;

            let mut ctx = zelf.builder();
            let key_path = keyfile
                .as_ref()
                .map(|path| path.to_path_buf(vm))
                .transpose()?;
            let cert_path = certfile.to_path_buf(vm)?;

            let file_error = |error: std::io::Error, path: &FsPath| {
                let exception = error.to_pyexception(vm);
                let filename: PyObjectRef = match path {
                    FsPath::Str(path) => path.clone().into(),
                    FsPath::Bytes(path) => path.clone().into(),
                };
                exception
                    .as_object()
                    .set_attr("filename", filename, vm)
                    .expect("OSError.filename is writable");
                exception
            };
            // Keep OS errors and their original str/bytes filenames rather
            // than replacing them with OpenSSL's generic file error.
            rustpython_host_env::fs::metadata(&cert_path)
                .map_err(|error| file_error(error, &certfile))?;
            if let Some(ref path) = key_path {
                rustpython_host_env::fs::metadata(path)
                    .map_err(|error| file_error(error, keyfile.as_ref().unwrap()))?;
            }

            // Load certificate chain
            ctx.set_certificate_chain_file(&cert_path)
                .map_err(|e| convert_openssl_error(vm, e))?;

            // Load private key - handle password if provided
            let key_file_path = key_path.as_ref().unwrap_or(&cert_path);

            // PEM_BUFSIZE = 1024 (maximum password length in OpenSSL)
            const PEM_BUFSIZE: usize = 1024;

            // Read key file data
            let key_data = rustpython_host_env::fs::read(key_file_path)
                .map_err(|error| file_error(error, keyfile.as_ref().unwrap_or(&certfile)))?;

            let pkey = if let Some(ref pw_obj) = password {
                if pw_obj.is_callable() {
                    // Callable password - use callback that calls Python function
                    // Store any Python error that occurs in the callback
                    let py_error: RefCell<Option<PyBaseExceptionRef>> = RefCell::new(None);

                    let result = PKey::private_key_from_pem_callback(&key_data, |buf| {
                        // Call the Python password callback
                        let pw_result = pw_obj.call((), vm);
                        match pw_result {
                            Ok(result) => {
                                // Extract password bytes
                                match Self::extract_password_bytes(
                                    &result,
                                    "password callback must return a string",
                                    vm,
                                ) {
                                    Ok(pw) => {
                                        // Check password length
                                        if pw.len() > PEM_BUFSIZE {
                                            *py_error.borrow_mut() =
                                                Some(vm.new_value_error(format!(
                                                    "password cannot be longer than {} bytes",
                                                    PEM_BUFSIZE
                                                )));
                                            return Err(openssl::error::ErrorStack::get());
                                        }
                                        let len = core::cmp::min(pw.len(), buf.len());
                                        buf[..len].copy_from_slice(&pw[..len]);
                                        Ok(len)
                                    }
                                    Err(e) => {
                                        *py_error.borrow_mut() = Some(e);
                                        Err(openssl::error::ErrorStack::get())
                                    }
                                }
                            }
                            Err(e) => {
                                *py_error.borrow_mut() = Some(e);
                                Err(openssl::error::ErrorStack::get())
                            }
                        }
                    });

                    // Check for Python error first
                    if let Some(py_err) = py_error.into_inner() {
                        return Err(py_err);
                    }

                    result.map_err(|e| convert_openssl_error(vm, e))?
                } else {
                    // Direct password (string/bytes)
                    let pw = Self::extract_password_bytes(
                        pw_obj,
                        "password should be a string or bytes",
                        vm,
                    )?;

                    // Check password length
                    if pw.len() > PEM_BUFSIZE {
                        return Err(vm.new_value_error(format!(
                            "password cannot be longer than {} bytes",
                            PEM_BUFSIZE
                        )));
                    }

                    PKey::private_key_from_pem_passphrase(&key_data, &pw)
                        .map_err(|e| convert_openssl_error(vm, e))?
                }
            } else {
                // No password - use SSL_CTX_use_PrivateKey_file directly for correct error messages
                ctx.set_private_key_file(key_file_path, ssl::SslFiletype::PEM)
                    .map_err(|e| convert_openssl_error(vm, e))?;

                // Verify key matches certificate and return early
                return ctx
                    .check_private_key()
                    .map_err(|e| convert_openssl_error(vm, e));
            };

            ctx.set_private_key(&pkey)
                .map_err(|e| convert_openssl_error(vm, e))?;

            // Verify key matches certificate
            ctx.check_private_key()
                .map_err(|e| convert_openssl_error(vm, e))
        }

        // Helper to extract password bytes from string/bytes/bytearray
        fn extract_password_bytes(
            obj: &PyObject,
            bad_type_error: &str,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<u8>> {
            use crate::vm::builtins::{PyByteArray, PyBytes, PyStr};

            if let Some(s) = obj.downcast_ref::<PyStr>() {
                Ok(s.as_bytes().to_vec())
            } else if let Some(b) = obj.downcast_ref::<PyBytes>() {
                Ok(b.as_bytes().to_vec())
            } else if let Some(ba) = obj.downcast_ref::<PyByteArray>() {
                Ok(ba.borrow_buf().to_vec())
            } else {
                Err(vm.new_type_error(bad_type_error.to_owned()))
            }
        }

        // Helper function to create SSL socket
        // = CPython's newPySSLSocket()
        fn new_py_ssl_socket(
            ctx_ref: PyRef<PySslContext>,
            server_side: bool,
            server_hostname: Option<PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(ssl::Ssl, SslServerOrClient, Option<PyStrRef>)> {
            // Validate socket type and context protocol
            if server_side && ctx_ref.protocol == SslVersion::TlsClient {
                return Err(new_ssl_error(
                    vm,
                    "Cannot create a server socket with a PROTOCOL_TLS_CLIENT context",
                ));
            }
            if !server_side && ctx_ref.protocol == SslVersion::TlsServer {
                return Err(new_ssl_error(
                    vm,
                    "Cannot create a client socket with a PROTOCOL_TLS_SERVER context",
                ));
            }

            // Create SSL object
            let mut ssl =
                ssl::Ssl::new(&ctx_ref.ctx()).map_err(|e| convert_openssl_error(vm, e))?;

            // Set session id context for server-side sockets
            let socket_type = if server_side {
                unsafe {
                    const SID_CTX: &[u8] = b"Python";
                    let ret = SSL_set_session_id_context(
                        ssl.as_ptr(),
                        SID_CTX.as_ptr(),
                        SID_CTX.len() as core::ffi::c_uint,
                    );
                    if ret == 0 {
                        return Err(convert_openssl_error(vm, ErrorStack::get()));
                    }
                }
                SslServerOrClient::Server
            } else {
                SslServerOrClient::Client
            };

            // Configure server hostname
            if let Some(hostname) = &server_hostname {
                if hostname.contains_nuls() {
                    cold_path();
                    return Err(exceptions::nul_char_type_error(vm));
                }
                let hostname_str: &str = hostname.as_ref();
                if hostname_str.is_empty() || hostname_str.starts_with('.') {
                    return Err(vm.new_value_error(
                        "server_hostname cannot be an empty string or start with a leading dot.",
                    ));
                }
                let ip = hostname_str.parse::<core::net::IpAddr>();
                if ip.is_err() {
                    ssl.set_hostname(hostname_str)
                        .map_err(|e| convert_openssl_error(vm, e))?;
                }
                if ctx_ref.check_hostname.load() {
                    if let Ok(ip) = ip {
                        ssl.param_mut()
                            .set_ip(ip)
                            .map_err(|e| convert_openssl_error(vm, e))?;
                    } else {
                        ssl.param_mut()
                            .set_host(hostname_str)
                            .map_err(|e| convert_openssl_error(vm, e))?;
                    }
                }
            }

            // Configure post-handshake authentication
            #[cfg(ossl111)]
            if *ctx_ref.post_handshake_auth.lock() {
                unsafe {
                    if server_side {
                        // Server socket: add SSL_VERIFY_POST_HANDSHAKE flag
                        // Only in combination with SSL_VERIFY_PEER
                        let mode = sys::SSL_get_verify_mode(ssl.as_ptr());
                        if (mode & sys::SSL_VERIFY_PEER as core::ffi::c_int) != 0 {
                            sys::SSL_set_verify(
                                ssl.as_ptr(),
                                mode | SSL_VERIFY_POST_HANDSHAKE,
                                None,
                            );
                        }
                    } else {
                        // Client socket: call SSL_set_post_handshake_auth
                        SSL_set_post_handshake_auth(ssl.as_ptr(), 1);
                    }
                }
            }

            // Set connect/accept state
            if server_side {
                ssl.set_accept_state();
            } else {
                ssl.set_connect_state();
            }

            Ok((ssl, socket_type, server_hostname))
        }

        #[pymethod]
        fn _wrap_socket(
            zelf: PyRef<Self>,
            args: WrapSocketArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            // Use common helper function
            let (ssl, socket_type, server_hostname) =
                Self::new_py_ssl_socket(zelf.clone(), args.server_side, args.server_hostname, vm)?;

            // Create SslStream with socket
            let stream = ssl::SslStream::new(
                ssl,
                SocketStream::new(args.sock.clone()).map_err(|e| e.to_pyexception(vm))?,
            )
            .map_err(|e| convert_openssl_error(vm, e))?;

            let py_ssl_socket = PySslSocket {
                ctx: PyRwLock::new(zelf.clone()),
                connection: SslConnectionLock(PyDetachingRwLock::new(SslConnection::new(
                    SslTransport::Socket(stream),
                ))),
                socket_type,
                server_hostname,
                got_eof_error: AtomicCell::new(false),
                shutdown_seen_zero: AtomicCell::new(false),
                callback_error: PyMutex::new(None),
                #[cfg(all(unix, not(target_os = "aix")))]
                info_progress: AtomicCell::new(0),
                #[cfg(all(unix, not(target_os = "aix")))]
                previous_info_callback: AtomicCell::new(None),
                owner: PyRwLock::new(Some(
                    args.owner
                        .unwrap_or_else(|| args.sock.clone().into())
                        .downgrade(None, vm)?,
                )),
            };

            // Convert to PyRef (heap allocation) to avoid use-after-free
            let py_ref =
                py_ssl_socket.into_ref_with_type(vm, PySslSocket::class(&vm.ctx).to_owned())?;

            // The connection is owned by py_ref, so this borrowed address
            // remains valid through every SSL operation, including retries.
            unsafe {
                sys::SSL_set_ex_data(
                    py_ref.connection.read().ssl().as_ptr(),
                    get_socket_ex_data_index(),
                    &*py_ref as *const Py<PySslSocket> as *mut _,
                );
            }

            #[cfg(all(unix, not(target_os = "aix")))]
            py_ref.install_progress_callback();

            // Set session if provided
            if let Some(session) = args.session
                && !vm.is_none(&session)
            {
                PySslSocket::set_session(&py_ref, session, vm)?;
            }

            Ok(py_ref.into())
        }

        #[pymethod]
        fn _wrap_bio(
            zelf: PyRef<Self>,
            args: WrapBioArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            // Use common helper function
            let (ssl, socket_type, server_hostname) =
                Self::new_py_ssl_socket(zelf.clone(), args.server_side, args.server_hostname, vm)?;

            // Create BioStream wrapper
            let bio_stream = BioStream {
                inbio: args.incoming,
                outbio: args.outgoing,
                #[cfg(all(unix, not(target_os = "aix")))]
                progress: 0,
            };

            // Create SslStream with BioStream
            let stream =
                ssl::SslStream::new(ssl, bio_stream).map_err(|e| convert_openssl_error(vm, e))?;

            let py_ssl_socket = PySslSocket {
                ctx: PyRwLock::new(zelf.clone()),
                connection: SslConnectionLock(PyDetachingRwLock::new(SslConnection::new(
                    SslTransport::Bio(stream),
                ))),
                socket_type,
                server_hostname,
                got_eof_error: AtomicCell::new(false),
                shutdown_seen_zero: AtomicCell::new(false),
                callback_error: PyMutex::new(None),
                #[cfg(all(unix, not(target_os = "aix")))]
                info_progress: AtomicCell::new(0),
                #[cfg(all(unix, not(target_os = "aix")))]
                previous_info_callback: AtomicCell::new(None),
                owner: PyRwLock::new(args.owner.map(|o| o.downgrade(None, vm)).transpose()?),
            };

            // Convert to PyRef (heap allocation) to avoid use-after-free
            let py_ref =
                py_ssl_socket.into_ref_with_type(vm, PySslSocket::class(&vm.ctx).to_owned())?;

            // The connection is owned by py_ref, so this borrowed address
            // remains valid through every SSL operation, including retries.
            unsafe {
                sys::SSL_set_ex_data(
                    py_ref.connection.read().ssl().as_ptr(),
                    get_socket_ex_data_index(),
                    &*py_ref as *const Py<PySslSocket> as *mut _,
                );
            }

            #[cfg(all(unix, not(target_os = "aix")))]
            py_ref.install_progress_callback();

            // Set session if provided
            if let Some(session) = args.session
                && !vm.is_none(&session)
            {
                PySslSocket::set_session(&py_ref, session, vm)?;
            }

            Ok(py_ref.into())
        }
    }

    #[derive(FromArgs)]
    #[allow(dead_code)] // Fields will be used when _wrap_bio is fully implemented
    struct WrapBioArgs {
        incoming: PyRef<PySslMemoryBio>,
        outgoing: PyRef<PySslMemoryBio>,
        server_side: bool,
        #[pyarg(any, default)]
        server_hostname: Option<PyStrRef>,
        #[pyarg(named, default)]
        owner: Option<PyObjectRef>,
        #[pyarg(named, default)]
        session: Option<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct WrapSocketArgs {
        sock: PyRef<PySocket>,
        server_side: bool,
        #[pyarg(any, default)]
        server_hostname: Option<PyStrRef>,
        #[pyarg(named, default)]
        owner: Option<PyObjectRef>,
        #[pyarg(named, default)]
        session: Option<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct LoadVerifyLocationsArgs {
        #[pyarg(any, default)]
        cafile: Option<FsPath>,
        #[pyarg(any, default)]
        capath: Option<FsPath>,
        #[pyarg(any, default)]
        cadata: Option<Either<PyStrRef, ArgBytesLike>>,
    }

    #[derive(FromArgs)]
    struct LoadCertChainArgs {
        certfile: FsPath,
        #[pyarg(any, optional)]
        keyfile: Option<FsPath>,
        #[pyarg(any, optional)]
        password: Option<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct GetCertArgs {
        #[pyarg(any, optional)]
        binary_form: OptionalArg<bool>,
    }

    #[derive(FromArgs)]
    struct GetGroupsArgs {
        #[pyarg(named, default)]
        include_aliases: bool,
    }

    // Err is true if the socket is blocking
    type SocketDeadline = Result<Instant, bool>;

    #[cfg(not(all(unix, not(target_os = "aix"))))]
    enum SelectRet {
        Nonblocking,
        TimedOut,
        Closed,
        Ok,
    }

    #[cfg(not(all(unix, not(target_os = "aix"))))]
    #[derive(Clone, Copy)]
    enum SslNeeds {
        Read,
        Write,
    }

    struct SocketStream {
        socket: PyRef<PySocket>,
        #[cfg(all(unix, not(target_os = "aix")))]
        identity: Arc<socket::SocketIdentity>,
        #[cfg(all(unix, not(target_os = "aix")))]
        progress: u64,
        #[cfg(all(unix, not(target_os = "aix")))]
        interrupted: bool,
    }

    impl SocketStream {
        fn new(socket: PyRef<PySocket>) -> std::io::Result<Self> {
            Ok(Self {
                #[cfg(all(unix, not(target_os = "aix")))]
                identity: socket.ssl_identity()?,
                socket,
                #[cfg(all(unix, not(target_os = "aix")))]
                progress: 0,
                #[cfg(all(unix, not(target_os = "aix")))]
                interrupted: false,
            })
        }
        fn timeout_deadline(&self) -> SocketDeadline {
            self.socket.get_timeout().map(|d| Instant::now() + d)
        }

        #[cfg(not(all(unix, not(target_os = "aix"))))]
        fn select(
            &self,
            needs: SslNeeds,
            deadline: &SocketDeadline,
            vm: &VirtualMachine,
        ) -> PyResult<SelectRet> {
            let Some(sock) = self.socket.sock_opt() else {
                return Ok(SelectRet::Closed);
            };
            // For blocking sockets without timeout, call sock_select with None timeout
            // to actually block waiting for data instead of busy-looping
            let timeout = match &deadline {
                Ok(deadline) => match deadline.checked_duration_since(Instant::now()) {
                    Some(d) => Some(d),
                    None => return Ok(SelectRet::TimedOut),
                },
                Err(true) => None, // Blocking: no timeout, wait indefinitely
                Err(false) => return Ok(SelectRet::Nonblocking),
            };
            let wait_kind = match needs {
                SslNeeds::Read => SockWaitKind::Read,
                SslNeeds::Write => SockWaitKind::Write,
            };
            sock_wait(&*sock, wait_kind, timeout, vm).map(|timed_out| {
                if timed_out {
                    SelectRet::TimedOut
                } else {
                    SelectRet::Ok
                }
            })
        }

        #[cfg(not(all(unix, not(target_os = "aix"))))]
        fn socket_needs(
            &self,
            err: &ssl::Error,
            deadline: &SocketDeadline,
            vm: &VirtualMachine,
        ) -> PyResult<(Option<SslNeeds>, SelectRet)> {
            let needs = match err.code() {
                ssl::ErrorCode::WANT_READ => Some(SslNeeds::Read),
                ssl::ErrorCode::WANT_WRITE => Some(SslNeeds::Write),
                _ => None,
            };
            let state =
                needs.map_or(Ok(SelectRet::Ok), |needs| self.select(needs, deadline, vm))?;
            Ok((needs, state))
        }
    }

    fn socket_closed_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
        new_ssl_error(vm, "Underlying socket has been closed.")
    }

    // BIO stream wrapper to implement Read/Write traits for MemoryBIO
    struct BioStream {
        inbio: PyRef<PySslMemoryBio>,
        outbio: PyRef<PySslMemoryBio>,
        #[cfg(all(unix, not(target_os = "aix")))]
        progress: u64,
    }

    impl Read for BioStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            // Read from incoming MemoryBIO
            unsafe {
                let nbytes = sys::BIO_read(
                    self.inbio.bio,
                    buf.as_mut_ptr() as *mut _,
                    buf.len().min(i32::MAX as usize) as i32,
                );
                if nbytes < 0 {
                    // BIO_read returns -1 on error or when no data is available
                    // Check if it's a retry condition (WANT_READ)
                    Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "BIO has no data available",
                    ))
                } else {
                    #[cfg(all(unix, not(target_os = "aix")))]
                    {
                        self.progress = self.progress.wrapping_add(nbytes as u64);
                    }
                    Ok(nbytes as usize)
                }
            }
        }
    }

    impl Write for BioStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            // Write to outgoing MemoryBIO
            unsafe {
                let nbytes = sys::BIO_write(
                    self.outbio.bio,
                    buf.as_ptr() as *const _,
                    buf.len().min(i32::MAX as usize) as i32,
                );
                if nbytes < 0 {
                    return Err(std::io::Error::other("BIO write failed"));
                }
                #[cfg(all(unix, not(target_os = "aix")))]
                {
                    self.progress = self.progress.wrapping_add(nbytes as u64);
                }
                Ok(nbytes as usize)
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            // MemoryBIO doesn't need flushing
            Ok(())
        }
    }

    // Enum to represent different SSL connection modes
    enum SslTransport {
        Socket(ssl::SslStream<SocketStream>),
        Bio(ssl::SslStream<BioStream>),
    }

    impl Drop for SslTransport {
        fn drop(&mut self) {
            // Match CPython: freeing a socket without unwrap() must not invalidate
            // a session retained by Python. This marks state without doing I/O.
            let state = self.get_shutdown() | ssl::ShutdownState::SENT;
            match self {
                Self::Socket(stream) => stream.set_shutdown(state),
                Self::Bio(stream) => stream.set_shutdown(state),
            }
        }
    }

    impl SslTransport {
        // Get a reference to the SSL object
        fn ssl(&self) -> &ssl::SslRef {
            match self {
                SslTransport::Socket(stream) => stream.ssl(),
                SslTransport::Bio(stream) => stream.ssl(),
            }
        }

        // Get underlying socket stream reference (only for socket mode)
        fn get_ref(&self) -> Option<&SocketStream> {
            match self {
                SslTransport::Socket(stream) => Some(stream.get_ref()),
                SslTransport::Bio(_) => None,
            }
        }

        // Check if this is in BIO mode (legacy blocking transport).
        #[cfg(not(all(unix, not(target_os = "aix"))))]
        fn is_bio(&self) -> bool {
            matches!(self, SslTransport::Bio(_))
        }

        // Perform SSL handshake
        fn do_handshake(&mut self) -> Result<(), ssl::Error> {
            // SSL_get_error requires an empty queue before this native attempt.
            unsafe { sys::ERR_clear_error() };
            match self {
                SslTransport::Socket(stream) => stream.do_handshake(),
                SslTransport::Bio(stream) => stream.do_handshake(),
            }
        }

        // Write data to SSL connection
        fn ssl_write(&mut self, buf: &[u8]) -> Result<usize, ssl::Error> {
            unsafe { sys::ERR_clear_error() };
            match self {
                SslTransport::Socket(stream) => stream.ssl_write(buf),
                SslTransport::Bio(stream) => stream.ssl_write(buf),
            }
        }

        // Read data from SSL connection
        fn ssl_read(&mut self, buf: &mut [u8]) -> Result<usize, ssl::Error> {
            unsafe { sys::ERR_clear_error() };
            match self {
                SslTransport::Socket(stream) => stream.ssl_read(buf),
                SslTransport::Bio(stream) => stream.ssl_read(buf),
            }
        }

        fn shutdown(&mut self) -> Result<ssl::ShutdownResult, ssl::Error> {
            unsafe { sys::ERR_clear_error() };
            match self {
                SslTransport::Socket(stream) => stream.shutdown(),
                SslTransport::Bio(stream) => stream.shutdown(),
            }
        }

        // Get SSL shutdown state
        fn get_shutdown(&mut self) -> ssl::ShutdownState {
            match self {
                SslTransport::Socket(stream) => stream.get_shutdown(),
                SslTransport::Bio(stream) => stream.get_shutdown(),
            }
        }

        // Check if incoming BIO has EOF (for BIO mode only)
        fn is_bio_eof(&self) -> bool {
            match self {
                SslTransport::Socket(_) => false,
                SslTransport::Bio(stream) => stream.get_ref().inbio.eof_written.load(),
            }
        }
    }

    struct SslConnection {
        transport: SslTransport,
        #[cfg(all(unix, not(target_os = "aix")))]
        driver: Driver,
        #[cfg(all(unix, not(target_os = "aix")))]
        progress: Arc<ProgressRegistry>,
    }

    impl SslConnection {
        fn new(transport: SslTransport) -> Self {
            Self {
                transport,
                #[cfg(all(unix, not(target_os = "aix")))]
                driver: Driver::default(),
                #[cfg(all(unix, not(target_os = "aix")))]
                progress: Arc::new(ProgressRegistry::default()),
            }
        }

        #[cfg(all(unix, not(target_os = "aix")))]
        fn notify_progress(&self) {
            self.progress.notify();
        }

        #[cfg(all(unix, not(target_os = "aix")))]
        fn io_progress(&self) -> u64 {
            match &self.transport {
                SslTransport::Socket(stream) => stream.get_ref().progress,
                SslTransport::Bio(stream) => stream.get_ref().progress,
            }
        }

        #[cfg(all(unix, not(target_os = "aix")))]
        fn take_interrupted(&mut self) -> bool {
            match &mut self.transport {
                SslTransport::Socket(stream) => core::mem::take(&mut stream.get_mut().interrupted),
                SslTransport::Bio(_) => false,
            }
        }
    }

    impl core::ops::Deref for SslConnection {
        type Target = SslTransport;
        fn deref(&self) -> &Self::Target {
            &self.transport
        }
    }

    impl core::ops::DerefMut for SslConnection {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.transport
        }
    }

    struct SslConnectionLock(PyDetachingRwLock<SslConnection>);

    impl core::ops::Deref for SslConnectionLock {
        type Target = PyDetachingRwLock<SslConnection>;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    // Like Traverse for PyRwLock, conservatively retain a connection if a
    // detached SSL operation holds its write guard when GC stops the world.
    unsafe impl Traverse for SslConnectionLock {
        fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
            if let Some(connection) = self.try_read() {
                match &connection.transport {
                    SslTransport::Socket(stream) => stream.get_ref().socket.traverse(traverse_fn),
                    SslTransport::Bio(stream) => {
                        stream.get_ref().inbio.traverse(traverse_fn);
                        stream.get_ref().outbio.traverse(traverse_fn);
                    }
                }
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "_ssl", name = "_SSLSocket", traverse)]
    #[derive(PyPayload)]
    struct PySslSocket {
        ctx: PyRwLock<PyRef<PySslContext>>,
        connection: SslConnectionLock,
        #[pytraverse(skip)]
        socket_type: SslServerOrClient,
        server_hostname: Option<PyStrRef>,
        owner: PyRwLock<Option<PyRef<PyWeak>>>,
        #[pytraverse(skip)]
        got_eof_error: AtomicCell<bool>,
        #[pytraverse(skip)]
        shutdown_seen_zero: AtomicCell<bool>,
        callback_error: PyMutex<Option<PyBaseExceptionRef>>,
        #[cfg(all(unix, not(target_os = "aix")))]
        #[pytraverse(skip)]
        info_progress: AtomicCell<u64>,
        #[cfg(all(unix, not(target_os = "aix")))]
        #[pytraverse(skip)]
        previous_info_callback: AtomicCell<SslInfoCallback>,
    }

    impl fmt::Debug for PySslSocket {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("_SSLSocket")
        }
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    struct IoScope(*const PySslSocket);

    #[cfg(all(unix, not(target_os = "aix")))]
    impl IoScope {
        fn enter(socket: &PySslSocket, vm: &VirtualMachine) -> PyResult<Self> {
            let pointer = socket as *const _;
            let entered = SSL_IO_CALLS.with(|calls| {
                let mut calls = calls.borrow_mut();
                if calls.contains(&pointer) {
                    false
                } else {
                    calls.push(pointer);
                    true
                }
            });
            // Exception allocation can run Python finalizers. Do not retain a
            // RefCell borrow while they may attempt another SSL invocation.
            if entered {
                Ok(Self(pointer))
            } else {
                Err(vm.new_runtime_error("Reentrant I/O on the same SSL connection"))
            }
        }
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    impl Drop for IoScope {
        fn drop(&mut self) {
            SSL_IO_CALLS.with(|calls| {
                let pointer = calls.borrow_mut().pop();
                debug_assert_eq!(pointer, Some(self.0));
            });
        }
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    enum IoOperation<'a> {
        Handshake,
        Read(&'a mut [u8]),
        Write(Option<Box<[u8]>>),
        Shutdown,
        #[cfg(ossl111)]
        PostHandshake,
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    impl IoOperation<'_> {
        fn timeout_message(&self, needs: Option<Needs>) -> &'static str {
            match self {
                Self::Handshake => "The handshake operation timed out",
                Self::Read(_) => "The read operation timed out",
                Self::Write(_) => "The write operation timed out",
                Self::Shutdown if needs == Some(Needs::Read) => "The read operation timed out",
                Self::Shutdown => "The write operation timed out",
                #[cfg(ossl111)]
                Self::PostHandshake => "The handshake operation timed out",
            }
        }
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    enum IoOutput {
        Done,
        Count(usize),
        Shutdown(ssl::ShutdownResult),
        Unwrapped(Option<PyRef<PySocket>>),
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    enum IoStep {
        Done(IoOutput),
        // Preserve the pre-existing clean-read-EOF/PHA signal boundary.
        Immediate(IoOutput),
        Again,
        Wait(Option<std::os::fd::RawFd>, Needs),
    }

    // Owned failures leave the native guard before exception allocation or
    // attribute writes, either of which can execute Python (including GC).
    #[cfg(all(unix, not(target_os = "aix")))]
    enum IoError {
        Eof,
        Closed,
        Timeout(&'static str),
        WantWrite,
        Message(&'static str),
        Io(std::io::Error),
        Callback(PyBaseExceptionRef),
        Native(SslErrorSnapshot, Option<PyBaseExceptionRef>),
        #[cfg(ossl111)]
        Stack(ErrorStack, Option<PyBaseExceptionRef>),
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    impl IoError {
        fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            match self {
                Self::Eof => create_ssl_eof_error(vm).upcast(),
                Self::Closed => socket_closed_error(vm),
                Self::Timeout(message) => {
                    socket::timeout_error_msg(vm, message.to_owned()).upcast()
                }
                Self::WantWrite => create_ssl_want_write_error(vm).upcast(),
                Self::Message(message) => new_ssl_error(vm, message),
                Self::Io(error) => error.to_pyexception(vm),
                Self::Callback(error) => error,
                Self::Native(error, callback) => {
                    let error = error.into_pyexception(vm);
                    if let Some(callback) = callback {
                        error.set_context(Some(callback));
                    }
                    error
                }
                #[cfg(ossl111)]
                Self::Stack(error, callback) => {
                    let error = convert_openssl_error(vm, error);
                    if let Some(callback) = callback {
                        error.set_context(Some(callback));
                    }
                    error
                }
            }
        }
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    struct IoCall<'a> {
        _ssl: &'a PySslSocket,
        owner: Option<WriteOwner>,
        owns_pending: bool,
        last_needs: Option<Needs>,
        progress: Arc<ProgressRegistry>,
        socket: Option<(PyRef<PySocket>, Arc<socket::SocketIdentity>, SocketDeadline)>,
        waiter: Option<Arc<ProgressWaiter>>,
        // Keep this installed through waits and signal callbacks, but never
        // expose a stale native SSL pointer while its guard is released.
        _scope: IoScope,
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    impl<'a> IoCall<'a> {
        fn new(ssl: &'a PySslSocket, writing: bool, vm: &VirtualMachine) -> PyResult<Self> {
            let scope = IoScope::enter(ssl, vm)?;
            let connection = ssl.connection.read();
            let progress = connection.progress.clone();
            let socket = connection.get_ref().map(|stream| {
                (
                    stream.socket.clone(),
                    stream.identity.clone(),
                    stream.timeout_deadline(),
                )
            });
            Ok(Self {
                _ssl: ssl,
                owner: writing.then(WriteOwner::new),
                owns_pending: false,
                last_needs: None,
                progress,
                socket,
                waiter: None,
                _scope: scope,
            })
        }

        fn prepare_wait(
            &mut self,
            observed: u64,
            needs: Option<Needs>,
            error: IoError,
            timeout_message: &'static str,
        ) -> Result<IoStep, IoError> {
            let Some((socket, identity, deadline)) = &self.socket else {
                return Err(error);
            };
            match deadline {
                Err(false) => return Err(error),
                Ok(deadline) if *deadline <= Instant::now() => {
                    return Err(IoError::Timeout(timeout_message));
                }
                _ => {}
            }
            if self.waiter.is_none() {
                self.waiter = Some(Arc::new(ProgressWaiter::new().map_err(IoError::Io)?));
            }
            let waiter = self.waiter.as_ref().expect("waiter initialized");
            // Close does not need the SSL lock. Drain BEFORE checking identity
            // and subscribing, or a concurrent close notification could be lost.
            match waiter.drain() {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                    return Ok(IoStep::Again);
                }
                Err(error) => return Err(IoError::Io(error)),
            }
            let fd = socket
                .sock_for_ssl_identity(identity)
                .map_err(|_| IoError::Closed)?
                .as_raw_fd();
            if !identity.subscribe(waiter.clone()) {
                return Err(IoError::Closed);
            }
            self.progress.register(waiter, observed);
            // Exclusive-writer contenders must not poll socket writability: it
            // can remain ready while they are forbidden to execute native I/O.
            Ok(IoStep::Wait(
                needs.map(|_| fd),
                needs.unwrap_or(Needs::Write),
            ))
        }

        fn wait(
            &self,
            fd: Option<std::os::fd::RawFd>,
            needs: Needs,
            timeout_message: &'static str,
            vm: &VirtualMachine,
        ) -> Result<(), IoError> {
            let waiter = self.waiter.as_ref().expect("registered waiter");
            let deadline = self
                .socket
                .as_ref()
                .and_then(|(_, _, deadline)| deadline.as_ref().ok().copied());
            loop {
                let result = vm.allow_threads(|| waiter.wait(fd, needs == Needs::Write, deadline));
                // No connection or socket lock is held while Python signals run.
                vm.check_signals().map_err(IoError::Callback)?;
                match result {
                    Ok(false) => return Ok(()),
                    Ok(true) => {
                        return Err(IoError::Timeout(timeout_message));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        if self
                            .socket
                            .as_ref()
                            .is_some_and(|(_, identity, _)| identity.is_closed())
                        {
                            return Err(IoError::Closed);
                        }
                        return Err(IoError::Io(error));
                    }
                }
            }
        }

        fn abandon_pending(&mut self) {
            if self.owns_pending {
                self.owner.as_ref().expect("pending write owner").release();
                self.owns_pending = false;
                self.progress.notify();
            }
        }
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    impl Drop for IoCall<'_> {
        fn drop(&mut self) {
            // Cleanup must never acquire the SSL lock: a callback on another
            // thread may be waiting for this invocation to finish. Publish an
            // orphan before waking generation-checked registrations instead.
            self.abandon_pending();
            if let Some(waiter) = &self.waiter {
                self.progress.unregister(waiter);
                if let Some((_, identity, _)) = &self.socket {
                    identity.unsubscribe(waiter.clone());
                }
            }
        }
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    impl PySslSocket {
        fn install_progress_callback(&self) {
            let connection = self.connection.write();
            let pointer = connection.ssl().as_ptr();
            unsafe {
                self.previous_info_callback
                    .store(SSL_get_info_callback(pointer));
                SSL_set_info_callback(pointer, Some(ssl_info_progress));
            }
        }

        fn drive(&self, mut operation: IoOperation<'_>, vm: &VirtualMachine) -> PyResult<IoOutput> {
            self.check_eof(vm)?;
            let mut call = IoCall::new(self, matches!(operation, IoOperation::Write(_)), vm)?;
            let mut queued = false;
            let mut shutdown_zeros = 0;
            let mut attempted = false;
            let mut waited_for_ownership = false;
            loop {
                let step = (|| {
                    let mut connection = self.connection.write();
                    if self.got_eof_error.load() {
                        return Err(IoError::Eof);
                    }
                    if connection.driver.terminal {
                        return Err(IoError::Message(
                            "TLS connection is unusable after a fatal error",
                        ));
                    }
                    if call
                        .socket
                        .as_ref()
                        .is_some_and(|(_, identity, _)| identity.is_closed())
                    {
                        return Err(IoError::Closed);
                    }
                    // Buffered TLS data can satisfy even a very short positive
                    // timeout. Attempt native I/O once before timing retries.
                    if (attempted || waited_for_ownership)
                        && call.socket.as_ref().is_some_and(|(_, _, deadline)| {
                            deadline
                                .as_ref()
                                .is_ok_and(|deadline| *deadline <= Instant::now())
                        })
                    {
                        return Err(IoError::Timeout(operation.timeout_message(call.last_needs)));
                    }
                    // Abandonment is deliberately independent of the SSL lock.
                    // Observe its generation before checking the predicate.
                    let observed = call.progress.generation();
                    let permit = match &mut operation {
                        IoOperation::Write(bytes) => connection.driver.write(
                            call.owner.as_ref().unwrap(),
                            bytes,
                            &mut queued,
                        ),
                        _ => connection.driver.other_io(),
                    };
                    match permit {
                        Permit::RetryOriginalWrite => Err(IoError::WantWrite),
                        Permit::InvalidRetry => Err(IoError::Message(
                            "The pending SSL write must be retried with the same data in a new call",
                        )),
                        Permit::Wait => {
                            waited_for_ownership = true;
                            call.prepare_wait(
                                observed,
                                None,
                                IoError::WantWrite,
                                operation.timeout_message(call.last_needs),
                            )
                        }
                        Permit::Ready => {
                            attempted = true;
                            call.owns_pending = call.owner.is_some();
                            let before = (
                                connection.io_progress(),
                                self.info_progress.load(),
                                connection.get_shutdown(),
                            );
                            let permission_before = connection.driver.other_io();
                            let pointer = connection.ssl().as_ptr();
                            let result = {
                                let _native_scope =
                                    HandshakeVmGuard::new(vm, &self.connection, pointer);
                                match &mut operation {
                                    IoOperation::Handshake => vm
                                        .allow_threads(|| connection.do_handshake())
                                        .map(|()| IoOutput::Done),
                                    IoOperation::Read(buffer) => vm
                                        .allow_threads(|| connection.ssl_read(buffer))
                                        .map(IoOutput::Count),
                                    IoOperation::Write(_) => {
                                        let SslConnection {
                                            transport, driver, ..
                                        } = &mut *connection;
                                        vm.allow_threads(|| {
                                            transport.ssl_write(
                                                driver.bytes(call.owner.as_ref().unwrap()),
                                            )
                                        })
                                        .map(IoOutput::Count)
                                    }
                                    IoOperation::Shutdown => {
                                        if self.shutdown_seen_zero.load() {
                                            // Leave plaintext after close_notify for unwrap().
                                            unsafe {
                                                sys::SSL_ctrl(
                                                    pointer,
                                                    sys::SSL_CTRL_SET_READ_AHEAD,
                                                    0,
                                                    core::ptr::null_mut(),
                                                );
                                            }
                                        }
                                        vm.allow_threads(|| connection.shutdown())
                                            .map(IoOutput::Shutdown)
                                    }
                                    #[cfg(ossl111)]
                                    IoOperation::PostHandshake => {
                                        unsafe { sys::ERR_clear_error() };
                                        let result = vm.allow_threads(|| unsafe {
                                            SSL_verify_client_post_handshake(pointer)
                                        });
                                        let error = (result == 0).then(ErrorStack::get);
                                        let callback = self.callback_error.lock().take();
                                        connection.notify_progress();
                                        return match (error, callback) {
                                            (Some(error), callback) => {
                                                Err(IoError::Stack(error, callback))
                                            }
                                            (None, Some(callback)) => {
                                                Err(IoError::Callback(callback))
                                            }
                                            (None, None) => Ok(IoStep::Immediate(IoOutput::Done)),
                                        };
                                    }
                                }
                            };
                            let needs =
                                result.as_ref().err().and_then(|error| match error.code() {
                                    ssl::ErrorCode::WANT_READ => Some(Needs::Read),
                                    ssl::ErrorCode::WANT_WRITE => Some(Needs::Write),
                                    _ => None,
                                });
                            call.last_needs = needs;
                            let terminal = result.as_ref().err().is_some_and(|error| {
                                matches!(
                                    error.code(),
                                    ssl::ErrorCode::SSL | ssl::ErrorCode::SYSCALL
                                )
                            });
                            // Settle native ownership BEFORE a callback/signal can
                            // return an exception. Success must never be replayed.
                            if let Some(owner) = &call.owner {
                                call.owns_pending = connection.driver.settle_write(owner, needs);
                            }
                            if terminal {
                                connection.driver.fail();
                            }
                            if result.as_ref().err().is_some_and(is_unexpected_eof) {
                                self.got_eof_error.store(true);
                            }
                            let after = (
                                connection.io_progress(),
                                self.info_progress.load(),
                                connection.get_shutdown(),
                            );
                            if before != after
                                || result.is_ok()
                                || terminal
                                || permission_before != connection.driver.other_io()
                            {
                                connection.notify_progress();
                            }
                            let interrupted = connection.take_interrupted();
                            // Snapshot exactly once, while this operation still
                            // owns native state. No unlocked helper drains it.
                            let callback = self.callback_error.lock().take();
                            if let Some(callback) = callback {
                                // A fully blocking socket could not expose this
                                // intermediate WANT before the nonblocking driver.
                                // Timeout/nonblocking sockets and MemoryBIO retain
                                // native-error-first exception chaining.
                                if needs.is_some()
                                    && call.socket.as_ref().is_some_and(|(_, _, deadline)| {
                                        matches!(deadline, Err(true))
                                    })
                                {
                                    return Err(IoError::Callback(callback));
                                }
                                return Err(match result {
                                    Ok(_) => IoError::Callback(callback),
                                    Err(error) => IoError::Native(
                                        SslErrorSnapshot::capture(error, pointer),
                                        Some(callback),
                                    ),
                                });
                            }
                            match result {
                                Ok(IoOutput::Shutdown(ssl::ShutdownResult::Sent)) => {
                                    shutdown_zeros += 1;
                                    self.shutdown_seen_zero.store(true);
                                    if shutdown_zeros > 1 {
                                        Ok(IoStep::Done(IoOutput::Unwrapped(
                                            call.socket
                                                .as_ref()
                                                .map(|(socket, _, _)| socket.clone()),
                                        )))
                                    } else {
                                        Ok(IoStep::Again)
                                    }
                                }
                                Ok(IoOutput::Shutdown(ssl::ShutdownResult::Received)) => {
                                    Ok(IoStep::Done(IoOutput::Unwrapped(
                                        call.socket.as_ref().map(|(socket, _, _)| socket.clone()),
                                    )))
                                }
                                Ok(output) => Ok(IoStep::Done(output)),
                                Err(error) => {
                                    // Callback errors take precedence over these
                                    // successful/EOF read reinterpretations above.
                                    if matches!(operation, IoOperation::Read(_)) {
                                        if error.code() == ssl::ErrorCode::ZERO_RETURN {
                                            return Ok(IoStep::Immediate(IoOutput::Count(0)));
                                        }
                                        if error.code() == ssl::ErrorCode::WANT_READ
                                            && connection.is_bio_eof()
                                        {
                                            self.got_eof_error.store(true);
                                            connection.driver.fail();
                                            connection.notify_progress();
                                            return Err(IoError::Eof);
                                        }
                                    }
                                    let error = IoError::Native(
                                        SslErrorSnapshot::capture(error, pointer),
                                        None,
                                    );
                                    if let Some(needs) = needs {
                                        if interrupted {
                                            Ok(IoStep::Again)
                                        } else {
                                            call.prepare_wait(
                                                observed,
                                                Some(needs),
                                                error,
                                                operation.timeout_message(call.last_needs),
                                            )
                                        }
                                    } else {
                                        Err(error)
                                    }
                                }
                            }
                        }
                    }
                })();
                // Allocation/attribute writes may run arbitrary Python. The
                // SSL guard and callback pointer are gone, and a known-exiting
                // write no longer blocks contenders as an active owner.
                let step = match step {
                    Ok(IoStep::Immediate(output)) => return Ok(output),
                    Ok(step) => step,
                    Err(error) => {
                        call.abandon_pending();
                        return Err(error.into_pyexception(vm));
                    }
                };
                if let Err(error) = vm.check_signals() {
                    call.abandon_pending();
                    return Err(error);
                }
                match step {
                    IoStep::Immediate(_) => unreachable!("immediate result already returned"),
                    IoStep::Done(output) => return Ok(output),
                    IoStep::Again => {}
                    IoStep::Wait(fd, needs) => {
                        if let Err(error) =
                            call.wait(fd, needs, operation.timeout_message(call.last_needs), vm)
                        {
                            call.abandon_pending();
                            return Err(error.into_pyexception(vm));
                        }
                    }
                }
            }
        }
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    type SslInfoCallback =
        Option<unsafe extern "C" fn(*const sys::SSL, core::ffi::c_int, core::ffi::c_int)>;

    #[cfg(all(unix, not(target_os = "aix")))]
    unsafe extern "C" {
        fn SSL_get_info_callback(ssl: *const sys::SSL) -> SslInfoCallback;
        fn SSL_set_info_callback(ssl: *mut sys::SSL, callback: SslInfoCallback);
        fn SSL_CTX_get_info_callback(ctx: *const sys::SSL_CTX) -> SslInfoCallback;
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    unsafe extern "C" fn ssl_info_progress(
        ssl: *const sys::SSL,
        where_: core::ffi::c_int,
        value: core::ffi::c_int,
    ) {
        // These public callback bits are present in OpenSSL 1.1.1 as well as 3.x.
        const SSL_CB_LOOP: core::ffi::c_int = 0x01;
        const SSL_CB_HANDSHAKE_DONE: core::ffi::c_int = 0x20;
        const SSL_CB_ALERT: core::ffi::c_int = 0x4000;
        unsafe {
            let socket =
                sys::SSL_get_ex_data(ssl, get_socket_ex_data_index()) as *const Py<PySslSocket>;
            if socket.is_null() {
                return;
            }
            let socket = &*socket;
            if where_ & (SSL_CB_LOOP | SSL_CB_HANDSHAKE_DONE | SSL_CB_ALERT) != 0 {
                socket
                    .info_progress
                    .store(socket.info_progress.load().wrapping_add(1));
            }
            // A NULL per-SSL callback delegates to the CURRENT context, including
            // a context installed by SNI. Preserve that fallback dynamically.
            let callback = socket
                .previous_info_callback
                .load()
                .or_else(|| SSL_CTX_get_info_callback(sys::SSL_get_SSL_CTX(ssl)));
            if let Some(callback) = callback {
                callback(ssl, where_, value);
            }
        }
    }

    #[pyclass(flags(IMMUTABLETYPE))]
    impl PySslSocket {
        #[cfg(all(unix, not(target_os = "aix")))]
        #[pymethod]
        fn do_handshake(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            zelf.drive(IoOperation::Handshake, vm).map(|_| ())
        }

        #[cfg(all(unix, not(target_os = "aix")))]
        #[pymethod]
        fn shutdown(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Option<PyRef<PySocket>>> {
            // Do not reacquire the connection after completion: another native
            // callback may already be waiting for this Python call to return.
            match zelf.drive(IoOperation::Shutdown, vm)? {
                IoOutput::Unwrapped(socket) => Ok(socket),
                _ => unreachable!("shutdown returns its underlying socket"),
            }
        }

        #[cfg(all(unix, not(target_os = "aix")))]
        #[pymethod]
        fn verify_client_post_handshake(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            cfg_select! {
                ossl111 => zelf.drive(IoOperation::PostHandshake, vm).map(|_| ()),
                _ => Err(vm.new_not_implemented_error("Post-handshake auth is not supported by your OpenSSL version.")),
            }
        }

        #[cfg(all(unix, not(target_os = "aix")))]
        #[pymethod]
        fn write(zelf: &Py<Self>, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            zelf.check_eof(vm)?;
            // OpenSSL retains the write across WANT, including after this
            // Python invocation returns. Own the bytes, not an exporter lock.
            let bytes = {
                let source = data.borrow_buf();
                let mut owned = Vec::new();
                owned
                    .try_reserve_exact(source.len())
                    .map_err(|_| vm.no_memory_error())?;
                owned.extend_from_slice(&source);
                owned.into_boxed_slice()
            };
            match zelf.drive(IoOperation::Write(Some(bytes)), vm)? {
                IoOutput::Count(count) => Ok(count),
                _ => unreachable!("write returns a byte count"),
            }
        }

        #[cfg(all(unix, not(target_os = "aix")))]
        #[pymethod]
        fn read(
            zelf: &Py<Self>,
            n: isize,
            buffer: OptionalArg<ArgMemoryBuffer>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let read_len = match &buffer {
                OptionalArg::Present(buffer) => {
                    let len = buffer.borrow_buf_mut().len();
                    if n <= 0 || n as usize > len {
                        len
                    } else {
                        n as usize
                    }
                }
                OptionalArg::Missing => {
                    if n < 0 {
                        return Err(vm.new_value_error("size should not be negative"));
                    }
                    n as usize
                }
            };
            zelf.check_eof(vm)?;
            // As in CPython, a zero-length read need not enter native SSL.
            if read_len == 0 {
                return Ok(if buffer.is_present() {
                    vm.ctx.new_int(0).into()
                } else {
                    vm.ctx.new_bytes(vec![]).into()
                });
            }
            // Do not keep the caller's buffer locked while waiting for a peer.
            let mut scratch = vm.new_zeroed_bytes(read_len)?;
            let count = match zelf.drive(IoOperation::Read(&mut scratch), vm)? {
                IoOutput::Count(count) => count,
                _ => unreachable!("read returns a byte count"),
            };
            Ok(match &buffer {
                OptionalArg::Present(buffer) => {
                    buffer.borrow_buf_mut()[..count].copy_from_slice(&scratch[..count]);
                    vm.ctx.new_int(count).into()
                }
                OptionalArg::Missing => {
                    scratch.truncate(count);
                    scratch.shrink_to_fit();
                    vm.ctx.new_bytes(scratch).into()
                }
            })
        }

        fn callback_owner(zelf: &Py<Self>) -> Option<PyObjectRef> {
            if let Some(owner) = zelf.owner.read().as_ref() {
                return owner.upgrade();
            }
            // A MemoryBIO wrapper may have no Python owner.
            Some(zelf.as_object().to_owned())
        }

        fn check_eof(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.got_eof_error.load() {
                Err(create_ssl_eof_error(vm).upcast())
            } else {
                Ok(())
            }
        }

        #[cfg(not(all(unix, not(target_os = "aix"))))]
        fn ssl_error(
            &self,
            error: ssl::Error,
            ssl_ptr: *const sys::SSL,
            vm: &VirtualMachine,
        ) -> PyBaseExceptionRef {
            if is_unexpected_eof(&error) {
                self.got_eof_error.store(true);
            }
            SslErrorSnapshot::capture(error, ssl_ptr).into_pyexception(vm)
        }

        #[cfg(not(all(unix, not(target_os = "aix"))))]
        fn finish_ssl_operation<T>(
            &self,
            result: Result<T, ssl::Error>,
            ssl_ptr: *const sys::SSL,
            vm: &VirtualMachine,
        ) -> PyResult<Result<T, ssl::Error>> {
            let callback_error = self.callback_error.lock().take();
            if let Some(callback_error) = callback_error {
                return Err(match result {
                    Ok(_) => callback_error,
                    Err(error) => {
                        let error = self.ssl_error(error, ssl_ptr, vm);
                        error.set_context(Some(callback_error));
                        error
                    }
                });
            }
            Ok(result)
        }

        #[pygetset]
        fn owner(zelf: &Py<Self>) -> Option<PyObjectRef> {
            zelf.owner.read().as_ref().and_then(|weak| weak.upgrade())
        }
        #[pygetset(setter)]
        fn set_owner(zelf: &Py<Self>, owner: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let mut lock = zelf.owner.write();
            lock.take();
            *lock = Some(owner.downgrade(None, vm)?);
            Ok(())
        }
        #[pygetset]
        fn server_side(zelf: &Py<Self>) -> bool {
            zelf.socket_type == SslServerOrClient::Server
        }
        #[pygetset]
        fn context(zelf: &Py<Self>) -> PyRef<PySslContext> {
            zelf.ctx.read().clone()
        }
        #[pygetset(setter)]
        fn set_context(
            zelf: &Py<Self>,
            value: PyRef<PySslContext>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            with_ssl_pointer_mut(&zelf.connection, |ssl_ptr| {
                unsafe {
                    let result = SSL_set_SSL_CTX(ssl_ptr, value.ctx().as_ptr());
                    if result.is_null() {
                        return Err(vm.new_runtime_error("Failed to set SSL context"));
                    }
                }
                *zelf.ctx.write() = value;
                Ok(())
            })
        }
        #[pygetset]
        fn server_hostname(zelf: &Py<Self>) -> Option<PyStrRef> {
            zelf.server_hostname.clone()
        }

        #[pymethod]
        fn getpeercert(
            zelf: &Py<Self>,
            args: GetCertArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let binary = args.binary_form.unwrap_or(false);
            with_ssl_pointer(&zelf.connection, |ssl_ptr| {
                let ssl = unsafe { ssl::SslRef::from_ptr(ssl_ptr) };
                if !ssl.is_init_finished() {
                    return Err(vm.new_value_error("handshake not done yet"));
                }

                let peer_cert = ssl.peer_certificate();
                let Some(cert) = peer_cert else {
                    return Ok(None);
                };

                if binary {
                    // Return DER-encoded certificate
                    cert_to_py(vm, &cert, true).map(Some)
                } else {
                    // Check verify_mode
                    unsafe {
                        let ssl_ctx = sys::SSL_get_SSL_CTX(ssl_ptr);
                        let verify_mode = sys::SSL_CTX_get_verify_mode(ssl_ctx);
                        if (verify_mode & sys::SSL_VERIFY_PEER as core::ffi::c_int) == 0 {
                            // Return empty dict when SSL_VERIFY_PEER is not set
                            Ok(Some(vm.ctx.new_dict().into()))
                        } else {
                            // Return decoded certificate
                            cert_to_py(vm, &cert, false).map(Some)
                        }
                    }
                }
            })
        }

        #[pymethod]
        fn get_unverified_chain(
            zelf: &Py<Self>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyListRef>> {
            with_ssl_pointer(&zelf.connection, |ssl_ptr| {
                let ssl = unsafe { ssl::SslRef::from_ptr(ssl_ptr) };
                let Some(chain) = ssl.peer_cert_chain() else {
                    return Ok(None);
                };

                // Return Certificate objects
                let mut certs: Vec<PyObjectRef> = chain
                    .iter()
                    .map(|cert| unsafe {
                        sys::X509_up_ref(cert.as_ptr());
                        let owned = X509::from_ptr(cert.as_ptr());
                        cert_to_certificate(vm, owned)
                    })
                    .collect::<PyResult<_>>()?;

                // SSL_get_peer_cert_chain does not include peer cert for server-side sockets
                // Add it manually at the beginning
                if matches!(zelf.socket_type, SslServerOrClient::Server)
                    && let Some(peer_cert) = ssl.peer_certificate()
                {
                    let peer_obj = cert_to_certificate(vm, peer_cert)?;
                    certs.insert(0, peer_obj);
                }

                Ok(Some(vm.ctx.new_list(certs)))
            })
        }

        #[pymethod]
        fn get_verified_chain(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Option<PyListRef>> {
            with_ssl_pointer(&zelf.connection, |ssl_ptr| {
                unsafe {
                    let chain = sys::SSL_get0_verified_chain(ssl_ptr);
                    if chain.is_null() {
                        return Ok(None);
                    }

                    let num_certs = sys::OPENSSL_sk_num(chain as *const _);

                    let mut certs = Vec::with_capacity(num_certs as usize);
                    // Return Certificate objects
                    for i in 0..num_certs {
                        let cert_ptr =
                            sys::OPENSSL_sk_value(chain as *const _, i) as *mut sys::X509;
                        if cert_ptr.is_null() {
                            continue;
                        }
                        // Clone the X509 certificate to create an owned copy
                        sys::X509_up_ref(cert_ptr);
                        let owned_cert = X509::from_ptr(cert_ptr);
                        let cert_obj = cert_to_certificate(vm, owned_cert)?;
                        certs.push(cert_obj);
                    }

                    Ok(if certs.is_empty() {
                        None
                    } else {
                        Some(vm.ctx.new_list(certs))
                    })
                }
            })
        }

        #[pymethod]
        fn version(zelf: &Py<Self>) -> Option<&'static str> {
            with_ssl_pointer(&zelf.connection, |ssl_ptr| {
                // Return None if handshake is not complete (CPython behavior).
                if unsafe { sys::SSL_is_init_finished(ssl_ptr) } == 0 {
                    return None;
                }
                let version = unsafe { ssl::SslRef::from_ptr(ssl_ptr).version_str() };
                if version == "unknown" {
                    None
                } else {
                    Some(version)
                }
            })
        }
        #[pymethod]
        fn cipher(zelf: &Py<Self>) -> Option<CipherTuple> {
            with_ssl_pointer(&zelf.connection, |ssl_ptr| {
                unsafe { ssl::SslRef::from_ptr(ssl_ptr).current_cipher() }.map(cipher_to_tuple)
            })
        }
        #[pymethod]
        fn group(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Option<String>> {
            #[cfg(ossl320)]
            {
                let _ = vm;
                Ok(with_ssl_pointer(&zelf.connection, |ssl_ptr| {
                    let name = unsafe { SSL_get0_group_name(ssl_ptr) };
                    (!name.is_null())
                        .then(|| unsafe { CStr::from_ptr(name).to_string_lossy().into_owned() })
                }))
            }
            #[cfg(not(ossl320))]
            {
                let _ = zelf;
                Err(vm.new_not_implemented_error(
                    "Getting selected group requires OpenSSL 3.2 or later.",
                ))
            }
        }

        fn signature_name(
            &self,
            side: SslServerOrClient,
            vm: &VirtualMachine,
        ) -> PyResult<Option<String>> {
            #[cfg(ossl350)]
            {
                let _ = vm;
                const SSL_CTRL_GET_SIGNATURE_NAME: core::ffi::c_int = 140;
                const SSL_CTRL_GET_PEER_SIGNATURE_NAME: core::ffi::c_int = 141;
                let command = if self.socket_type == side {
                    SSL_CTRL_GET_SIGNATURE_NAME
                } else {
                    SSL_CTRL_GET_PEER_SIGNATURE_NAME
                };
                Ok(with_ssl_pointer(&self.connection, |ssl_ptr| {
                    let mut name: *const core::ffi::c_char = core::ptr::null();
                    let result = unsafe {
                        sys::SSL_ctrl(ssl_ptr, command, 0, core::ptr::addr_of_mut!(name).cast())
                    };
                    (result != 0 && !name.is_null())
                        .then(|| unsafe { CStr::from_ptr(name).to_string_lossy().into_owned() })
                }))
            }
            #[cfg(not(ossl350))]
            {
                let _ = side;
                Err(vm.new_not_implemented_error(
                    "Getting sig algorithms requires OpenSSL 3.5 or later.",
                ))
            }
        }

        #[pymethod]
        fn client_sigalg(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Option<String>> {
            zelf.signature_name(SslServerOrClient::Client, vm)
        }

        #[pymethod]
        fn server_sigalg(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Option<String>> {
            zelf.signature_name(SslServerOrClient::Server, vm)
        }

        #[pymethod]
        fn uses_ktls_for_send(_zelf: &Py<Self>) -> bool {
            // SslStream uses a custom Rust BIO, never OpenSSL's kernel TLS socket BIO.
            false
        }

        #[pymethod]
        fn uses_ktls_for_recv(_zelf: &Py<Self>) -> bool {
            false
        }

        #[pymethod]
        fn pending(zelf: &Py<Self>) -> i32 {
            with_ssl_pointer(&zelf.connection, |ssl_ptr| unsafe {
                sys::SSL_pending(ssl_ptr)
            })
        }

        #[pymethod]
        fn shared_ciphers(zelf: &Py<Self>, vm: &VirtualMachine) -> Option<PyListRef> {
            #[cfg(ossl110)]
            {
                with_ssl_pointer(&zelf.connection, |ssl_ptr| {
                    unsafe {
                        let server_ciphers = SSL_get_ciphers(ssl_ptr);
                        if server_ciphers.is_null() {
                            return None;
                        }

                        let client_ciphers = SSL_get_client_ciphers(ssl_ptr);
                        if client_ciphers.is_null() {
                            return None;
                        }

                        let mut result = Vec::new();
                        let num_server = sys::OPENSSL_sk_num(server_ciphers as *const _);
                        let num_client = sys::OPENSSL_sk_num(client_ciphers as *const _);

                        for i in 0..num_server {
                            let server_cipher_ptr =
                                sys::OPENSSL_sk_value(server_ciphers as *const _, i)
                                    as *const sys::SSL_CIPHER;

                            // Check if client supports this cipher by comparing pointers
                            let mut found = false;
                            for j in 0..num_client {
                                let client_cipher_ptr =
                                    sys::OPENSSL_sk_value(client_ciphers as *const _, j)
                                        as *const sys::SSL_CIPHER;

                                if server_cipher_ptr == client_cipher_ptr {
                                    found = true;
                                    break;
                                }
                            }

                            if found {
                                let cipher =
                                    ssl::SslCipherRef::from_ptr(server_cipher_ptr as *mut _);
                                let (name, version, bits) = cipher_to_tuple(cipher);
                                let tuple = vm.new_tuple((
                                    vm.ctx.new_str(name),
                                    vm.ctx.new_str(version),
                                    vm.ctx.new_int(bits),
                                ));
                                result.push(tuple.into());
                            }
                        }

                        if result.is_empty() {
                            None
                        } else {
                            Some(vm.ctx.new_list(result))
                        }
                    }
                })
            }
            #[cfg(not(ossl110))]
            {
                let _ = vm;
                None
            }
        }

        #[pymethod]
        fn selected_alpn_protocol(zelf: &Py<Self>) -> Option<String> {
            #[cfg(ossl102)]
            {
                with_ssl_pointer(&zelf.connection, |ssl_ptr| unsafe {
                    let mut out: *const core::ffi::c_uchar = core::ptr::null();
                    let mut outlen: core::ffi::c_uint = 0;

                    sys::SSL_get0_alpn_selected(ssl_ptr, &mut out, &mut outlen);

                    if out.is_null() {
                        None
                    } else {
                        let slice = core::slice::from_raw_parts(out, outlen as usize);
                        Some(String::from_utf8_lossy(slice).into_owned())
                    }
                })
            }
            #[cfg(not(ossl102))]
            {
                None
            }
        }

        #[pymethod]
        fn get_channel_binding(
            zelf: &Py<Self>,
            cb_type: OptionalArg<PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyBytesRef>> {
            const CB_MAXLEN: usize = 512;

            let cb_type_str = cb_type.as_ref().map_or("tls-unique", |s| s.as_ref());

            if cb_type_str != "tls-unique" {
                return Err(vm.new_value_error(format!(
                    "Unsupported channel binding type '{}'",
                    cb_type_str
                )));
            }

            with_ssl_pointer(&zelf.connection, |ssl_ptr| {
                unsafe {
                    let session_reused = sys::SSL_session_reused(ssl_ptr) != 0;
                    let is_client = matches!(zelf.socket_type, SslServerOrClient::Client);

                    // Use XOR logic from CPython
                    let use_finished = session_reused ^ is_client;

                    let mut buf = vec![0u8; CB_MAXLEN];
                    let len = if use_finished {
                        sys::SSL_get_finished(ssl_ptr, buf.as_mut_ptr() as *mut _, CB_MAXLEN)
                    } else {
                        sys::SSL_get_peer_finished(ssl_ptr, buf.as_mut_ptr() as *mut _, CB_MAXLEN)
                    };

                    if len == 0 {
                        Ok(None)
                    } else {
                        buf.truncate(len);
                        Ok(Some(vm.ctx.new_bytes(buf)))
                    }
                }
            })
        }

        #[cfg(not(all(unix, not(target_os = "aix"))))]
        #[pymethod]
        fn verify_client_post_handshake(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            cfg_select! {
                ossl111 => {
                    with_ssl_pointer_mut(&zelf.connection, |ssl_ptr| {
                        let result = unsafe { SSL_verify_client_post_handshake(ssl_ptr) };
                        if result == 0 {
                            Err(convert_openssl_error(vm, openssl::error::ErrorStack::get()))
                        } else {
                            Ok(())
                        }
                    })
                }
                _ => Err(vm.new_not_implemented_error(
                    "Post-handshake auth is not supported by your OpenSSL version.",
                )),
            }
        }

        #[cfg(not(all(unix, not(target_os = "aix"))))]
        #[pymethod]
        fn shutdown(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Option<PyRef<PySocket>>> {
            let mut stream = zelf.connection.write();
            let ssl_ptr = stream.ssl().as_ptr();
            let _vm_guard = HandshakeVmGuard::new(vm, &zelf.connection, ssl_ptr);
            let deadline = stream.get_ref().map(SocketStream::timeout_deadline);
            let mut zeros = 0;
            loop {
                if zelf.shutdown_seen_zero.load() {
                    // Do not consume plaintext following the TLS close_notify.
                    unsafe {
                        sys::SSL_ctrl(
                            ssl_ptr,
                            sys::SSL_CTRL_SET_READ_AHEAD,
                            0,
                            core::ptr::null_mut(),
                        );
                    }
                }
                let error = match zelf.finish_ssl_operation(
                    vm.allow_threads(|| stream.shutdown()),
                    ssl_ptr,
                    vm,
                )? {
                    Ok(ssl::ShutdownResult::Received) => break,
                    Ok(ssl::ShutdownResult::Sent) => {
                        zeros += 1;
                        if zeros > 1 {
                            break;
                        }
                        zelf.shutdown_seen_zero.store(true);
                        // Retry immediately: the peer's notification may already
                        // be buffered, even when the transport is not readable.
                        continue;
                    }
                    Err(error) => error,
                };
                if let Some(socket) = stream.get_ref() {
                    let (needs, state) = socket.socket_needs(
                        &error,
                        deadline.as_ref().expect("socket deadline"),
                        vm,
                    )?;
                    match state {
                        SelectRet::TimedOut => {
                            let message = if error.code() == ssl::ErrorCode::WANT_READ {
                                "The read operation timed out"
                            } else {
                                "The write operation timed out"
                            };
                            return Err(socket::timeout_error_msg(vm, message.to_owned()).upcast());
                        }
                        SelectRet::Closed => return Err(socket_closed_error(vm)),
                        SelectRet::Ok if needs.is_some() => continue,
                        _ => {}
                    }
                }
                return Err(zelf.ssl_error(error, ssl_ptr, vm));
            }
            Ok(stream.get_ref().map(|socket| socket.socket.clone()))
        }

        #[cfg(osslconf = "OPENSSL_NO_COMP")]
        #[pymethod]
        fn compression(_zelf: &Py<Self>) -> Option<&'static str> {
            None
        }
        #[cfg(not(osslconf = "OPENSSL_NO_COMP"))]
        #[pymethod]
        fn compression(zelf: &Py<Self>) -> Option<&'static str> {
            with_ssl_pointer(&zelf.connection, |ssl_ptr| {
                let comp_method = unsafe { sys::SSL_get_current_compression(ssl_ptr) };
                if comp_method.is_null() {
                    return None;
                }
                let typ = unsafe { sys::COMP_get_type(comp_method) };
                let nid = Nid::from_raw(typ);
                if nid == Nid::UNDEF {
                    return None;
                }
                nid.short_name().ok()
            })
        }

        #[cfg(not(all(unix, not(target_os = "aix"))))]
        #[pymethod]
        fn do_handshake(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            zelf.check_eof(vm)?;
            let mut stream = zelf.connection.write();
            let ssl_ptr = stream.ssl().as_ptr();

            // Set up thread-local VM and SSL pointer for callbacks
            // This allows callbacks to access SSL without acquiring the connection lock
            let _vm_guard = HandshakeVmGuard::new(vm, &zelf.connection, ssl_ptr);

            // BIO mode: no timeout/select logic, just do handshake
            if stream.is_bio() {
                let result = zelf
                    .finish_ssl_operation(vm.allow_threads(|| stream.do_handshake()), ssl_ptr, vm)?
                    .map_err(|e| zelf.ssl_error(e, ssl_ptr, vm));
                return result;
            }

            // Socket mode: handle timeout and blocking
            let timeout = stream
                .get_ref()
                .expect("handshake called in bio mode; should only be called in socket mode")
                .timeout_deadline();
            loop {
                // On a blocking socket this waits for the peer, which may never
                // answer. `SSL_do_handshake` runs between
                // `Py_BEGIN_ALLOW_THREADS` and `Py_END_ALLOW_THREADS` for the
                // same reason. The connection lock stays held across it, which
                // is why it is a detaching lock: a thread reaching the same
                // socket gives up its interpreter rather than wait attached.
                let err = match zelf.finish_ssl_operation(
                    vm.allow_threads(|| stream.do_handshake()),
                    ssl_ptr,
                    vm,
                )? {
                    Ok(()) => {
                        return Ok(());
                    }
                    Err(e) => e,
                };
                let (needs, state) = stream
                    .get_ref()
                    .expect("handshake called in bio mode; should only be called in socket mode")
                    .socket_needs(&err, &timeout, vm)?;
                match state {
                    SelectRet::TimedOut => {
                        return Err(socket::timeout_error_msg(
                            vm,
                            "The handshake operation timed out".to_owned(),
                        )
                        .upcast());
                    }
                    SelectRet::Closed => {
                        return Err(socket_closed_error(vm));
                    }
                    SelectRet::Nonblocking => {}
                    SelectRet::Ok => {
                        // For blocking sockets, select() has completed successfully
                        // Continue the handshake loop (matches CPython's SOCKET_IS_BLOCKING behavior)
                        if needs.is_some() {
                            continue;
                        }
                    }
                }
                return Err(zelf.ssl_error(err, ssl_ptr, vm));
            }
        }

        #[cfg(not(all(unix, not(target_os = "aix"))))]
        #[pymethod]
        fn write(zelf: &Py<Self>, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            zelf.check_eof(vm)?;
            let mut stream = zelf.connection.write();
            let ssl_ptr = stream.ssl().as_ptr();
            let _vm_guard = HandshakeVmGuard::new(vm, &zelf.connection, ssl_ptr);
            let data = data.borrow_buf();
            let data = &*data;

            // BIO mode: no timeout/select logic
            if stream.is_bio() {
                return zelf
                    .finish_ssl_operation(vm.allow_threads(|| stream.ssl_write(data)), ssl_ptr, vm)?
                    .map_err(|e| zelf.ssl_error(e, ssl_ptr, vm));
            }

            // Socket mode: handle timeout and blocking
            let socket_ref = stream
                .get_ref()
                .expect("write called in bio mode; should only be called in socket mode");
            let timeout = socket_ref.timeout_deadline();
            let state = socket_ref.select(SslNeeds::Write, &timeout, vm)?;
            match state {
                SelectRet::TimedOut => {
                    return Err(socket::timeout_error_msg(
                        vm,
                        "The write operation timed out".to_owned(),
                    )
                    .upcast());
                }
                SelectRet::Closed => return Err(socket_closed_error(vm)),
                _ => {}
            }
            loop {
                // Sending waits for the peer to make room, which it need not
                // ever do; `SSL_write_ex` is released around for the same
                // reason.
                let err = match zelf.finish_ssl_operation(
                    vm.allow_threads(|| stream.ssl_write(data)),
                    ssl_ptr,
                    vm,
                )? {
                    Ok(len) => return Ok(len),
                    Err(e) => e,
                };
                let (needs, state) = stream
                    .get_ref()
                    .expect("write called in bio mode; should only be called in socket mode")
                    .socket_needs(&err, &timeout, vm)?;
                match state {
                    SelectRet::TimedOut => {
                        return Err(socket::timeout_error_msg(
                            vm,
                            "The write operation timed out".to_owned(),
                        )
                        .upcast());
                    }
                    SelectRet::Closed => return Err(socket_closed_error(vm)),
                    SelectRet::Nonblocking => {}
                    SelectRet::Ok => {
                        // For blocking sockets, select() has completed successfully
                        // Continue the write loop (matches CPython's SOCKET_IS_BLOCKING behavior)
                        if needs.is_some() {
                            continue;
                        }
                    }
                }
                return Err(zelf.ssl_error(err, ssl_ptr, vm));
            }
        }

        #[pygetset]
        fn session(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<Option<PySslSession>> {
            with_ssl_pointer(&zelf.connection, |ssl_ptr| {
                unsafe {
                    // Use SSL_get1_session which returns an owned reference (ref count already incremented)
                    let session_ptr = SSL_get1_session(ssl_ptr);
                    if session_ptr.is_null() {
                        Ok(None)
                    } else {
                        Ok(Some(PySslSession {
                            session: session_ptr,
                            ctx: zelf.ctx.read().clone(),
                        }))
                    }
                }
            })
        }

        #[pygetset(setter)]
        fn set_session(zelf: &Py<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            // Check if value is SSLSession type
            let session = value
                .downcast_ref::<PySslSession>()
                .ok_or_else(|| vm.new_type_error("Value is not a SSLSession."))?;

            // Check if session refers to the same SSLContext
            if !core::ptr::eq(
                zelf.ctx.read().ctx.read().as_ptr(),
                session.ctx.ctx.read().as_ptr(),
            ) {
                return Err(vm.new_value_error("Session refers to a different SSLContext."));
            }

            // Check if this is a client socket
            if zelf.socket_type != SslServerOrClient::Client {
                return Err(vm.new_value_error("Cannot set session for server-side SSLSocket."));
            }

            with_ssl_pointer_mut(&zelf.connection, |ssl_ptr| {
                unsafe {
                    if sys::SSL_is_init_finished(ssl_ptr) != 0 {
                        return Err(vm.new_value_error("Cannot set session after handshake."));
                    }
                    if sys::SSL_set_session(ssl_ptr, session.session) == 0 {
                        return Err(convert_openssl_error(vm, ErrorStack::get()));
                    }
                }
                Ok(())
            })
        }

        #[pygetset]
        fn session_reused(zelf: &Py<Self>) -> bool {
            with_ssl_pointer(&zelf.connection, |ssl_ptr| unsafe {
                sys::SSL_session_reused(ssl_ptr) != 0
            })
        }
        #[cfg(not(all(unix, not(target_os = "aix"))))]
        #[pymethod]
        fn read(
            zelf: &Py<Self>,
            n: isize,
            buffer: OptionalArg<ArgMemoryBuffer>,
            vm: &VirtualMachine,
        ) -> PyResult {
            // Handle negative n:
            // - If buffer is None and n < 0: raise ValueError
            // - If buffer is present and n <= 0: use buffer length
            // This matches _ssl__SSLSocket_read_impl in CPython
            let read_len: usize = match &buffer {
                OptionalArg::Present(buf) => {
                    let buf_len = buf.borrow_buf_mut().len();
                    if n <= 0 || (n as usize) > buf_len {
                        buf_len
                    } else {
                        n as usize
                    }
                }
                OptionalArg::Missing => {
                    if n < 0 {
                        return Err(vm.new_value_error("size should not be negative"));
                    }
                    n as usize
                }
            };

            zelf.check_eof(vm)?;

            // Special case: reading 0 bytes should return empty bytes immediately
            if read_len == 0 {
                return if buffer.is_present() {
                    Ok(vm.ctx.new_int(0).into())
                } else {
                    Ok(vm.ctx.new_bytes(vec![]).into())
                };
            }

            let mut stream = zelf.connection.write();
            let ssl_ptr = stream.ssl().as_ptr();
            let _vm_guard = HandshakeVmGuard::new(vm, &zelf.connection, ssl_ptr);
            // The read below answers when the peer writes, which may be never,
            // and reaching the caller's buffer takes a lock that every other
            // thread touching the same object waits on. Read aside and take
            // that lock only for the copy.
            let mut scratch = vm.new_zeroed_bytes(read_len)?;
            let buf = scratch.as_mut_slice();

            // BIO mode: no timeout/select logic
            let count = if stream.is_bio() {
                match zelf.finish_ssl_operation(
                    vm.allow_threads(|| stream.ssl_read(buf)),
                    ssl_ptr,
                    vm,
                )? {
                    Ok(count) => count,
                    Err(e) => {
                        // close_notify is an orderly TLS EOF.
                        if e.code() == ssl::ErrorCode::ZERO_RETURN {
                            return if buffer.is_present() {
                                Ok(vm.ctx.new_int(0).into())
                            } else {
                                Ok(vm.ctx.new_bytes(vec![]).into())
                            };
                        }
                        // If WANT_READ and the incoming BIO has EOF written,
                        // this is an unexpected EOF (transport closed without TLS close_notify)
                        if e.code() == ssl::ErrorCode::WANT_READ && stream.is_bio_eof() {
                            zelf.got_eof_error.store(true);
                            return Err(create_ssl_eof_error(vm).upcast());
                        }
                        return Err(zelf.ssl_error(e, ssl_ptr, vm));
                    }
                }
            } else {
                // Socket mode: handle timeout and blocking
                let timeout = stream
                    .get_ref()
                    .expect("read called in bio mode; should only be called in socket mode")
                    .timeout_deadline();
                loop {
                    // This is the wait the whole method is shaped around: it
                    // ends when the peer writes. `SSL_read_ex` is released
                    // around for the same reason.
                    let err = match zelf.finish_ssl_operation(
                        vm.allow_threads(|| stream.ssl_read(buf)),
                        ssl_ptr,
                        vm,
                    )? {
                        Ok(count) => break count,
                        Err(e) => e,
                    };
                    if err.code() == ssl::ErrorCode::ZERO_RETURN
                        && stream.get_shutdown() == ssl::ShutdownState::RECEIVED
                    {
                        break 0;
                    }
                    let (needs, state) = stream
                        .get_ref()
                        .expect("read called in bio mode; should only be called in socket mode")
                        .socket_needs(&err, &timeout, vm)?;
                    match state {
                        SelectRet::TimedOut => {
                            return Err(socket::timeout_error_msg(
                                vm,
                                "The read operation timed out".to_owned(),
                            )
                            .upcast());
                        }
                        SelectRet::Closed => return Err(socket_closed_error(vm)),
                        SelectRet::Nonblocking => {}
                        SelectRet::Ok => {
                            // For blocking sockets, select() has completed successfully
                            // Continue the read loop (matches CPython's SOCKET_IS_BLOCKING behavior)
                            if needs.is_some() {
                                continue;
                            }
                        }
                    }
                    return Err(zelf.ssl_error(err, ssl_ptr, vm));
                }
            };
            let ret = match &buffer {
                OptionalArg::Present(buffer) => {
                    buffer.borrow_buf_mut()[..count].copy_from_slice(&scratch[..count]);
                    vm.ctx.new_int(count).into()
                }
                OptionalArg::Missing => {
                    scratch.truncate(count);
                    scratch.shrink_to_fit();
                    vm.ctx.new_bytes(scratch).into()
                }
            };
            Ok(ret)
        }
    }

    #[pyattr]
    #[pyclass(module = "_ssl", name = "SSLSession", unhashable = true)]
    #[derive(PyPayload)]
    struct PySslSession {
        session: *mut sys::SSL_SESSION,
        ctx: PyRef<PySslContext>,
    }

    impl fmt::Debug for PySslSession {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("SSLSession")
        }
    }

    impl Drop for PySslSession {
        fn drop(&mut self) {
            if !self.session.is_null() {
                unsafe {
                    sys::SSL_SESSION_free(self.session);
                }
            }
        }
    }

    unsafe impl Send for PySslSession {}
    unsafe impl Sync for PySslSession {}

    impl Comparable for PySslSession {
        fn cmp(
            zelf: &Py<Self>,
            other: &crate::vm::PyObject,
            op: PyComparisonOp,
            _vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            let other = class_or_notimplemented!(Self, other);

            if !matches!(op, PyComparisonOp::Eq | PyComparisonOp::Ne) {
                return Ok(PyComparisonValue::NotImplemented);
            }
            let mut eq = unsafe {
                let mut self_len: core::ffi::c_uint = 0;
                let mut other_len: core::ffi::c_uint = 0;
                let self_id = sys::SSL_SESSION_get_id(zelf.session, &mut self_len);
                let other_id = sys::SSL_SESSION_get_id(other.session, &mut other_len);

                if self_len != other_len {
                    false
                } else {
                    let self_slice = core::slice::from_raw_parts(self_id, self_len as usize);
                    let other_slice = core::slice::from_raw_parts(other_id, other_len as usize);
                    self_slice == other_slice
                }
            };
            if matches!(op, PyComparisonOp::Ne) {
                eq = !eq;
            }
            Ok(PyComparisonValue::Implemented(eq))
        }
    }

    #[pyattr]
    #[pyclass(module = "_ssl", name = "MemoryBIO")]
    #[derive(PyPayload)]
    struct PySslMemoryBio {
        bio: *mut sys::BIO,
        eof_written: AtomicCell<bool>,
    }

    impl fmt::Debug for PySslMemoryBio {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("MemoryBIO")
        }
    }

    impl Drop for PySslMemoryBio {
        fn drop(&mut self) {
            if !self.bio.is_null() {
                unsafe {
                    sys::BIO_free_all(self.bio);
                }
            }
        }
    }

    unsafe impl Send for PySslMemoryBio {}
    unsafe impl Sync for PySslMemoryBio {}

    #[pyfunction]
    fn get_sigalgs(vm: &VirtualMachine) -> PyResult<PyListRef> {
        #[cfg(ossl340)]
        unsafe {
            let names = SSL_get1_builtin_sigalgs(core::ptr::null_mut());
            if names.is_null() {
                return Err(vm.new_memory_error("failed to get signature algorithms"));
            }
            let names_text = CStr::from_ptr(names).to_string_lossy().into_owned();
            sys::CRYPTO_free(names.cast(), core::ptr::null(), 0);
            Ok(vm.ctx.new_list(
                names_text
                    .split(':')
                    .map(|name| vm.ctx.new_str(name).into())
                    .collect(),
            ))
        }
        #[cfg(not(ossl340))]
        {
            Err(vm.new_not_implemented_error(
                "Getting signature algorithms requires OpenSSL 3.4 or later.",
            ))
        }
    }

    // OpenSSL functions not in openssl-sys

    #[cfg(ossl320)]
    unsafe extern "C" {
        fn SSL_get0_group_name(ssl: *mut sys::SSL) -> *const core::ffi::c_char;
    }

    #[cfg(ossl340)]
    unsafe extern "C" {
        fn SSL_get1_builtin_sigalgs(ctx: *mut core::ffi::c_void) -> *mut core::ffi::c_char;
    }

    unsafe extern "C" {
        // X509_check_ca returns 1 for CA certificates, 0 otherwise
        fn X509_check_ca(x: *const sys::X509) -> core::ffi::c_int;
    }

    unsafe extern "C" {
        fn SSL_get_ciphers(ssl: *const sys::SSL) -> *const sys::stack_st_SSL_CIPHER;
    }

    #[cfg(ossl110)]
    unsafe extern "C" {
        fn SSL_get_client_ciphers(ssl: *const sys::SSL) -> *const sys::stack_st_SSL_CIPHER;
    }

    #[cfg(ossl111)]
    unsafe extern "C" {
        fn SSL_verify_client_post_handshake(ssl: *const sys::SSL) -> core::ffi::c_int;
        fn SSL_set_post_handshake_auth(ssl: *mut sys::SSL, val: core::ffi::c_int);
    }

    #[cfg(ossl110)]
    unsafe extern "C" {
        fn SSL_CTX_get_security_level(ctx: *const sys::SSL_CTX) -> core::ffi::c_int;
    }

    unsafe extern "C" {
        fn SSL_set_SSL_CTX(ssl: *mut sys::SSL, ctx: *mut sys::SSL_CTX) -> *mut sys::SSL_CTX;
    }

    // Message callback type
    #[allow(non_camel_case_types)]
    type SSL_CTX_msg_callback = Option<
        unsafe extern "C" fn(
            write_p: core::ffi::c_int,
            version: core::ffi::c_int,
            content_type: core::ffi::c_int,
            buf: *const core::ffi::c_void,
            len: usize,
            ssl: *mut sys::SSL,
            arg: *mut core::ffi::c_void,
        ),
    >;

    unsafe extern "C" {
        fn SSL_CTX_set_msg_callback(ctx: *mut sys::SSL_CTX, cb: SSL_CTX_msg_callback);
    }

    #[cfg(ossl110)]
    unsafe extern "C" {
        fn SSL_SESSION_has_ticket(session: *const sys::SSL_SESSION) -> core::ffi::c_int;
        fn SSL_SESSION_get_ticket_lifetime_hint(
            session: *const sys::SSL_SESSION,
        ) -> core::ffi::c_ulong;
    }

    // X509 object types
    const X509_LU_X509: core::ffi::c_int = 1;
    const X509_LU_CRL: core::ffi::c_int = 2;

    unsafe extern "C" {
        fn X509_OBJECT_get_type(obj: *const sys::X509_OBJECT) -> core::ffi::c_int;
        fn SSL_set_session_id_context(
            ssl: *mut sys::SSL,
            sid_ctx: *const core::ffi::c_uchar,
            sid_ctx_len: core::ffi::c_uint,
        ) -> core::ffi::c_int;
        fn SSL_get1_session(ssl: *const sys::SSL) -> *mut sys::SSL_SESSION;
    }

    // SSL session statistics constants (used with SSL_CTX_ctrl)
    const SSL_CTRL_SESS_NUMBER: core::ffi::c_int = 20;
    const SSL_CTRL_SESS_CONNECT: core::ffi::c_int = 21;
    const SSL_CTRL_SESS_CONNECT_GOOD: core::ffi::c_int = 22;
    const SSL_CTRL_SESS_CONNECT_RENEGOTIATE: core::ffi::c_int = 23;
    const SSL_CTRL_SESS_ACCEPT: core::ffi::c_int = 24;
    const SSL_CTRL_SESS_ACCEPT_GOOD: core::ffi::c_int = 25;
    const SSL_CTRL_SESS_ACCEPT_RENEGOTIATE: core::ffi::c_int = 26;
    const SSL_CTRL_SESS_HIT: core::ffi::c_int = 27;
    const SSL_CTRL_SESS_MISSES: core::ffi::c_int = 29;
    const SSL_CTRL_SESS_TIMEOUTS: core::ffi::c_int = 30;
    const SSL_CTRL_SESS_CACHE_FULL: core::ffi::c_int = 31;

    // SSL session statistics functions (implemented as macros in OpenSSL)
    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_number(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_NUMBER,
                0,
                core::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_connect(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_CONNECT,
                0,
                core::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_connect_good(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_CONNECT_GOOD,
                0,
                core::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_connect_renegotiate(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_CONNECT_RENEGOTIATE,
                0,
                core::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_accept(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_ACCEPT,
                0,
                core::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_accept_good(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_ACCEPT_GOOD,
                0,
                core::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_accept_renegotiate(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_ACCEPT_RENEGOTIATE,
                0,
                core::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_hits(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe { sys::SSL_CTX_ctrl(ctx as *mut _, SSL_CTRL_SESS_HIT, 0, core::ptr::null_mut()) }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_misses(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_MISSES,
                0,
                core::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_timeouts(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_TIMEOUTS,
                0,
                core::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_cache_full(ctx: *const sys::SSL_CTX) -> core::ffi::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_CACHE_FULL,
                0,
                core::ptr::null_mut(),
            )
        }
    }

    // DH parameters functions
    unsafe extern "C" {
        fn PEM_read_DHparams(
            fp: *mut rustpython_host_env::fileutils::CFile,
            x: *mut *mut sys::DH,
            cb: *mut core::ffi::c_void,
            u: *mut core::ffi::c_void,
        ) -> *mut sys::DH;
    }

    // OpenSSL BIO helper functions
    // These are typically macros in OpenSSL, implemented via BIO_ctrl
    const BIO_CTRL_PENDING: core::ffi::c_int = 10;
    const BIO_CTRL_SET_EOF: core::ffi::c_int = 2;

    #[allow(non_snake_case)]
    unsafe fn BIO_ctrl_pending(bio: *mut sys::BIO) -> usize {
        unsafe { sys::BIO_ctrl(bio, BIO_CTRL_PENDING, 0, core::ptr::null_mut()) as usize }
    }

    #[allow(non_snake_case)]
    unsafe fn BIO_set_mem_eof_return(
        bio: *mut sys::BIO,
        eof: core::ffi::c_int,
    ) -> core::ffi::c_int {
        unsafe {
            sys::BIO_ctrl(
                bio,
                BIO_CTRL_SET_EOF,
                eof as core::ffi::c_long,
                core::ptr::null_mut(),
            ) as core::ffi::c_int
        }
    }

    #[allow(non_snake_case)]
    unsafe fn BIO_clear_retry_flags(bio: *mut sys::BIO) {
        unsafe {
            sys::BIO_clear_flags(bio, sys::BIO_FLAGS_RWS | sys::BIO_FLAGS_SHOULD_RETRY);
        }
    }

    impl Constructor for PySslMemoryBio {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            unsafe {
                let bio = sys::BIO_new(sys::BIO_s_mem());
                if bio.is_null() {
                    return Err(vm.new_memory_error("failed to allocate BIO"));
                }

                sys::BIO_set_retry_read(bio);
                BIO_set_mem_eof_return(bio, -1);

                Ok(PySslMemoryBio {
                    bio,
                    eof_written: AtomicCell::new(false),
                })
            }
        }
    }

    #[pyclass(flags(IMMUTABLETYPE), with(Constructor))]
    impl PySslMemoryBio {
        #[pygetset]
        fn pending(zelf: &Py<Self>) -> usize {
            unsafe { BIO_ctrl_pending(zelf.bio) }
        }

        #[pygetset]
        fn eof(zelf: &Py<Self>) -> bool {
            let pending = unsafe { BIO_ctrl_pending(zelf.bio) };
            pending == 0 && zelf.eof_written.load()
        }

        #[pymethod]
        fn read(zelf: &Py<Self>, size: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            unsafe {
                let avail = BIO_ctrl_pending(zelf.bio).min(i32::MAX as usize) as i32;
                let len = size.unwrap_or(-1);
                let len = if len < 0 || len > avail { avail } else { len };

                // When no data available, return empty bytes (CPython behavior)
                // CPython returns empty bytes directly without calling BIO_read()
                if len == 0 {
                    return Ok(Vec::new());
                }

                let mut buf = vec![0u8; len as usize];
                let nbytes = sys::BIO_read(zelf.bio, buf.as_mut_ptr() as *mut _, len);

                if nbytes < 0 {
                    return Err(convert_openssl_error(vm, ErrorStack::get()));
                }

                buf.truncate(nbytes as usize);
                buf.shrink_to_fit();
                Ok(buf)
            }
        }

        #[pymethod]
        fn write(zelf: &Py<Self>, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<i32> {
            if zelf.eof_written.load() {
                return Err(new_ssl_error(vm, "cannot write() after write_eof()"));
            }

            data.with_ref(|buf| unsafe {
                if buf.len() > i32::MAX as usize {
                    return Err(
                        vm.new_overflow_error(format!("string longer than {} bytes", i32::MAX))
                    );
                }

                let nbytes = sys::BIO_write(zelf.bio, buf.as_ptr() as *const _, buf.len() as i32);
                if nbytes < 0 {
                    return Err(convert_openssl_error(vm, ErrorStack::get()));
                }

                Ok(nbytes)
            })
        }

        #[pymethod]
        fn write_eof(zelf: &Py<Self>) {
            zelf.eof_written.store(true);
            unsafe {
                BIO_clear_retry_flags(zelf.bio);
                BIO_set_mem_eof_return(zelf.bio, 0);
            }
        }
    }

    #[pyclass(flags(IMMUTABLETYPE), with(Comparable))]
    impl PySslSession {
        #[pygetset]
        fn time(zelf: &Py<Self>) -> i64 {
            cfg_select! {
                ossl330 => unsafe { sys::SSL_SESSION_get_time(zelf.session) as i64 },
                _ => unsafe { sys::SSL_SESSION_get_time(zelf.session) as i64 },
            }
        }

        #[pygetset]
        fn timeout(zelf: &Py<Self>) -> i64 {
            unsafe { sys::SSL_SESSION_get_timeout(zelf.session) as i64 }
        }

        #[pygetset]
        fn ticket_lifetime_hint(zelf: &Py<Self>) -> u64 {
            // SSL_SESSION_get_ticket_lifetime_hint available in OpenSSL 1.1.0+
            cfg_select! {
                ossl110 => unsafe { SSL_SESSION_get_ticket_lifetime_hint(zelf.session) as u64 },
                // Not available in older OpenSSL versions
                _ => 0,
            }
        }

        #[pygetset]
        fn id(zelf: &Py<Self>, vm: &VirtualMachine) -> PyBytesRef {
            unsafe {
                let mut len: core::ffi::c_uint = 0;
                let id_ptr = sys::SSL_SESSION_get_id(zelf.session, &mut len);
                let id_slice = core::slice::from_raw_parts(id_ptr, len as usize);
                vm.ctx.new_bytes(id_slice.to_vec())
            }
        }

        #[pygetset]
        fn has_ticket(zelf: &Py<Self>) -> bool {
            // SSL_SESSION_has_ticket available in OpenSSL 1.1.0+
            cfg_select! {
                ossl110 => unsafe { SSL_SESSION_has_ticket(zelf.session) != 0 },
                // Not available in older OpenSSL versions
                _ => false,
            }
        }
    }

    /// Helper function to create SSL error with proper OSError subtype handling
    fn new_ssl_error(vm: &VirtualMachine, msg: impl ToString) -> PyBaseExceptionRef {
        vm.new_os_subtype_error(PySSLError::class(&vm.ctx).to_owned(), None, msg.to_string())
            .upcast()
    }

    fn system_error_errno(code: core::ffi::c_ulong) -> core::ffi::c_int {
        // OpenSSL 3 stores errno below ERR_SYSTEM_FLAG, as specified by
        // https://github.com/openssl/openssl/blob/openssl-3.5.7/include/openssl/err.h.in
        // openssl-sys 0.9.117's ERR_GET_REASON returns ERR_LIB_SYS instead.
        cfg_select! {
            ossl300 => (code & sys::ERR_SYSTEM_MASK) as core::ffi::c_int,
            _ => sys::ERR_GET_REASON(code),
        }
    }

    #[track_caller]
    pub(crate) fn convert_openssl_error(
        vm: &VirtualMachine,
        err: ErrorStack,
    ) -> PyBaseExceptionRef {
        convert_openssl_error_with_errno(vm, err, None)
    }

    #[track_caller]
    fn convert_openssl_error_with_errno(
        vm: &VirtualMachine,
        err: ErrorStack,
        operation_errno: Option<i32>,
    ) -> PyBaseExceptionRef {
        match err.errors().last() {
            Some(e) => {
                // Check if this is a system library error (errno-based)
                let lib = sys::ERR_GET_LIB(e.code());

                if lib == sys::ERR_LIB_SYS {
                    // A system error is being reported; reason is set to errno
                    let reason = system_error_errno(e.code());
                    let message = rustpython_host_env::errno::strerror_string(reason)
                        .unwrap_or_else(|| format!("System error {reason}"));

                    // errno 2 = ENOENT = FileNotFoundError
                    let exc_type = if reason == 2 {
                        vm.ctx.exceptions.file_not_found_error.to_owned()
                    } else {
                        vm.ctx.exceptions.os_error.to_owned()
                    };
                    return vm
                        .new_os_subtype_error(exc_type, Some(reason), message)
                        .upcast();
                }

                let caller = std::panic::Location::caller();
                let (file, line) = (caller.file(), caller.line());
                let file = file
                    .rsplit_once(&['/', '\\'][..])
                    .map_or(file, |(_, basename)| basename);

                // Get error codes - same approach as CPython
                let lib = sys::ERR_GET_LIB(e.code());
                let reason = sys::ERR_GET_REASON(e.code());

                // Look up error mnemonic from our static tables
                // CPython uses dict lookup: err_codes_to_names[(lib, reason)]
                let key = super::ssl_data::encode_error_key(lib, reason);
                let errstr = super::ssl_data::ERROR_CODES
                    .get(&key)
                    .copied()
                    .or_else(|| {
                        // Fallback: use OpenSSL's error string
                        e.reason()
                    })
                    .unwrap_or("unknown error");

                // Check if this is a certificate verification error
                // ERR_LIB_SSL = 20 (from _ssl_data_300.h)
                // SSL_R_CERTIFICATE_VERIFY_FAILED = 134 (from _ssl_data_300.h)
                let is_cert_verify_error = lib == 20 && reason == 134;

                // Look up library name from our static table
                // CPython uses: lib_codes_to_names[lib]
                let lib_name = super::ssl_data::LIBRARY_CODES.get(&(lib as u32)).copied();

                // Use SSLCertVerificationError for certificate verification failures
                let cls = if is_cert_verify_error {
                    PySSLCertVerificationError::class(&vm.ctx).to_owned()
                } else {
                    PySSLError::class(&vm.ctx).to_owned()
                };

                let description = e.reason().unwrap_or("unknown error");
                let msg = if let Some(lib_str) = lib_name {
                    format!("[{lib_str}: {errstr}] {description} ({file}:{line})")
                } else {
                    format!("{description} ({file}:{line})")
                };

                // Create exception instance
                let reason = sys::ERR_GET_REASON(e.code());
                let exc =
                    vm.new_os_subtype_error(cls, Some(operation_errno.unwrap_or(reason)), msg);
                let exc_obj: PyObjectRef = exc.upcast::<PyBaseException>().into();

                // Set reason attribute (always set, even if just the error string)
                let reason_value = vm.ctx.new_str(errstr);
                let _ = exc_obj.set_attr("reason", reason_value, vm);

                // Set library attribute (None if not available)
                let library_value: PyObjectRef = if let Some(lib_str) = lib_name {
                    vm.ctx.new_str(lib_str).into()
                } else {
                    vm.ctx.none()
                };
                let _ = exc_obj.set_attr("library", library_value, vm);

                // For SSLCertVerificationError, set verify_code and verify_message
                // Note: These will be set to None here, and can be updated by the caller
                // if they have access to the SSL object
                if is_cert_verify_error {
                    let _ = exc_obj.set_attr("verify_code", vm.ctx.none(), vm);
                    let _ = exc_obj.set_attr("verify_message", vm.ctx.none(), vm);
                }

                // Convert back to PyBaseExceptionRef
                exc_obj.downcast().expect(
                    "exc_obj is created as PyBaseExceptionRef and must downcast successfully",
                )
            }
            None => {
                let cls = PySSLError::class(&vm.ctx).to_owned();
                vm.new_os_subtype_error(cls, None, "unknown SSL error")
                    .upcast()
            }
        }
    }

    struct VerifyErrorInfo {
        code: core::ffi::c_long,
        message: Option<String>,
    }

    // Only owned metadata may outlive the native SSL guard. In particular,
    // exception normalization must never consult a later operation's SSL state.
    struct SslErrorSnapshot {
        error: ssl::Error,
        verify: Option<VerifyErrorInfo>,
    }

    impl SslErrorSnapshot {
        fn capture(error: ssl::Error, ssl_ptr: *const sys::SSL) -> Self {
            let is_cert_verify =
                matches!(error.code(), ssl::ErrorCode::SSL | ssl::ErrorCode::SYSCALL)
                    && error.ssl_error().is_some_and(|stack| {
                        stack.errors().last().is_some_and(|error| {
                            sys::ERR_GET_LIB(error.code()) == 20
                                && sys::ERR_GET_REASON(error.code()) == 134
                        })
                    });
            let verify = is_cert_verify.then(|| {
                // The caller holds this connection's native guard throughout
                // capture. Neither the pointer nor borrowed C data is retained.
                let code = unsafe { sys::SSL_get_verify_result(ssl_ptr) };
                let message = unsafe {
                    let pointer = sys::X509_verify_cert_error_string(code);
                    (!pointer.is_null()).then(|| {
                        core::ffi::CStr::from_ptr(pointer)
                            .to_string_lossy()
                            .into_owned()
                    })
                };
                VerifyErrorInfo { code, message }
            });
            Self { error, verify }
        }

        fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            let exception = convert_ssl_error(vm, self.error);
            if let Some(verify) = self.verify {
                let _ =
                    exception
                        .as_object()
                        .set_attr("verify_code", vm.ctx.new_int(verify.code), vm);
                let message = verify
                    .message
                    .map_or_else(|| vm.ctx.none(), |message| vm.ctx.new_str(message).into());
                let _ = exception
                    .as_object()
                    .set_attr("verify_message", message, vm);
            }
            exception
        }
    }

    fn is_unexpected_eof(error: &ssl::Error) -> bool {
        (error.code() == ssl::ErrorCode::SYSCALL
            && error.io_error().is_none()
            && error.ssl_error().is_none())
            || error.ssl_error().is_some_and(|stack| {
                stack.errors().last().is_some_and(|error| {
                    error.library_code() == ERR_LIB_SSL
                        && error.reason_code() == SSL_R_UNEXPECTED_EOF_WHILE_READING
                })
            })
    }

    #[track_caller]
    fn convert_ssl_error(
        vm: &VirtualMachine,
        e: impl std::borrow::Borrow<ssl::Error>,
    ) -> PyBaseExceptionRef {
        let e = e.borrow();
        if is_unexpected_eof(e) {
            return create_ssl_eof_error(vm).upcast();
        }

        let (cls, msg) = match e.code() {
            ssl::ErrorCode::WANT_READ => {
                return create_ssl_want_read_error(vm).upcast();
            }
            ssl::ErrorCode::WANT_WRITE => {
                return create_ssl_want_write_error(vm).upcast();
            }
            ssl::ErrorCode::SYSCALL => match e.io_error() {
                Some(io_err) => return io_err.to_pyexception(vm),
                None => {
                    if let Some(stack) = e.ssl_error() {
                        return convert_openssl_error_with_errno(
                            vm,
                            stack.clone(),
                            Some(sys::SSL_ERROR_SYSCALL),
                        );
                    }
                    return create_ssl_eof_error(vm).upcast();
                }
            },
            ssl::ErrorCode::SSL => {
                if let Some(ssl_err) = e.ssl_error() {
                    return convert_openssl_error_with_errno(
                        vm,
                        ssl_err.clone(),
                        Some(sys::SSL_ERROR_SSL),
                    );
                }
                (
                    PySSLError::class(&vm.ctx).to_owned(),
                    "A failure in the SSL library occurred",
                )
            }
            _ => (
                PySSLError::class(&vm.ctx).to_owned(),
                "A failure in the SSL library occurred",
            ),
        };
        vm.new_os_subtype_error(cls, None, msg).upcast()
    }

    // SSL_FILETYPE_ASN1 part of _add_ca_certs in CPython
    fn x509_stack_from_der(der: &[u8]) -> Result<Vec<X509>, ErrorStack> {
        unsafe {
            openssl::init();
            let bio = bio::MemBioSlice::new(der)?;

            let mut certs = vec![];
            let mut was_bio_eof = false;

            loop {
                // Check for EOF before attempting to parse (like CPython's _add_ca_certs)
                // BIO_ctrl with BIO_CTRL_EOF returns 1 if EOF, 0 otherwise
                if sys::BIO_ctrl(bio.as_ptr(), sys::BIO_CTRL_EOF, 0, core::ptr::null_mut()) != 0 {
                    was_bio_eof = true;
                    break;
                }

                let cert = sys::d2i_X509_bio(bio.as_ptr(), core::ptr::null_mut());
                if cert.is_null() {
                    // Parse error (not just EOF)
                    break;
                }
                certs.push(X509::from_ptr(cert));
            }

            // If we loaded some certs but didn't reach EOF, there's garbage data
            // (like cacert_der + b"A") - this is an error
            if !certs.is_empty() && !was_bio_eof {
                // Return the error from the last failed parse attempt
                return Err(ErrorStack::get());
            }

            // Clear any errors (including parse errors when no certs loaded)
            // Let the caller decide how to handle empty results
            sys::ERR_clear_error();
            Ok(certs)
        }
    }

    type CipherTuple = (&'static str, &'static str, i32);

    fn cipher_to_tuple(cipher: &ssl::SslCipherRef) -> CipherTuple {
        (cipher.name(), cipher.version(), cipher.bits().secret)
    }

    fn cipher_description(cipher: *const sys::SSL_CIPHER) -> String {
        unsafe {
            // SSL_CIPHER_description writes up to 128 bytes
            let mut buf = vec![0u8; 256];
            let result = sys::SSL_CIPHER_description(
                cipher,
                buf.as_mut_ptr() as *mut core::ffi::c_char,
                buf.len() as i32,
            );
            if result.is_null() {
                return String::from("No description available");
            }
            // Find the null terminator
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..len]).trim().to_string()
        }
    }

    #[cfg(all(unix, not(target_os = "aix")))]
    impl SocketStream {
        fn finish_transport(&mut self, result: std::io::Result<usize>) -> std::io::Result<usize> {
            match result {
                Ok(count) => {
                    self.progress = self.progress.wrapping_add(count as u64);
                    Ok(count)
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                    // openssl's custom BIO treats only WouldBlock/NotConnected
                    // as retryable. Passing EINTR through would become a fatal
                    // SSL_ERROR_SYSCALL. Check Python signals after settlement.
                    self.interrupted = true;
                    Err(std::io::Error::from(std::io::ErrorKind::WouldBlock))
                }
                Err(error) => Err(error),
            }
        }
    }

    impl Read for SocketStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            #[cfg(all(unix, not(target_os = "aix")))]
            {
                let result = {
                    let socket = self.socket.sock_for_ssl_identity(&self.identity)?;
                    // recv initializes only the reported prefix; this caller's
                    // buffer was already initialized and is valid throughout.
                    let buffer = unsafe {
                        core::slice::from_raw_parts_mut(
                            buf.as_mut_ptr().cast::<core::mem::MaybeUninit<u8>>(),
                            buf.len(),
                        )
                    };
                    socket.recv_with_flags(buffer, libc::MSG_DONTWAIT)
                };
                self.finish_transport(result)
            }
            #[cfg(not(all(unix, not(target_os = "aix"))))]
            {
                let mut socket: &PySocket = &self.socket;
                socket.read(buf)
            }
        }
    }

    impl Write for SocketStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            #[cfg(all(unix, not(target_os = "aix")))]
            {
                let result = {
                    let socket = self.socket.sock_for_ssl_identity(&self.identity)?;
                    socket.send_with_flags(buf, libc::MSG_DONTWAIT)
                };
                self.finish_transport(result)
            }
            #[cfg(not(all(unix, not(target_os = "aix"))))]
            {
                let mut socket: &PySocket = &self.socket;
                socket.write(buf)
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[cfg(target_os = "android")]
    mod android {
        use super::convert_openssl_error;
        use crate::vm::{VirtualMachine, builtins::PyBaseExceptionRef};
        use openssl::{
            ssl::SslContextBuilder,
            x509::{X509, store::X509StoreBuilder},
        };
        use rustpython_host_env::fs;
        use std::path::Path;

        static CERT_DIR: &'static str = "/system/etc/security/cacerts";

        pub(super) fn load_client_ca_list(
            vm: &VirtualMachine,
            b: &mut SslContextBuilder,
        ) -> Result<(), PyBaseExceptionRef> {
            let root = Path::new(CERT_DIR);
            if !fs::is_dir(root) {
                return Err(vm
                    .new_os_subtype_error(
                        vm.ctx.exceptions.file_not_found_error.to_owned(),
                        None,
                        CERT_DIR.to_string(),
                    )
                    .upcast());
            }

            let mut combined_pem = String::new();
            let entries = fs::read_dir(root)
                .map_err(|err| vm.new_os_error(format!("read cert root: {}", err)))?;
            for entry in entries {
                let entry =
                    entry.map_err(|err| vm.new_os_error(format!("iter cert root: {}", err)))?;

                let path = entry.path();
                if !fs::is_file(&path) {
                    continue;
                }

                let pem = fs::read_to_string(&path).map_err(|err| {
                    vm.new_os_error(format!("open cert file {}: {}", path.display(), err))
                })?;
                combined_pem.push_str(&pem);
                combined_pem.push('\n');
            }

            let mut store_b =
                X509StoreBuilder::new().map_err(|err| convert_openssl_error(vm, err))?;
            let x509_vec = X509::stack_from_pem(combined_pem.as_bytes())
                .map_err(|err| convert_openssl_error(vm, err))?;
            for x509 in x509_vec {
                store_b
                    .add_cert(x509)
                    .map_err(|err| convert_openssl_error(vm, err))?;
            }
            b.set_cert_store(store_b.build());

            Ok(())
        }
    }
    #[cfg(test)]
    mod error_tests {
        use super::{sys, system_error_errno};

        #[test]
        fn system_errors_preserve_errno() {
            for errno in [libc::ENOENT, libc::EPIPE, libc::ECONNRESET] {
                let code = cfg_select! {
                    ossl300 => sys::ERR_SYSTEM_FLAG | errno as core::ffi::c_ulong,
                    _ => sys::ERR_PACK(sys::ERR_LIB_SYS, 0, errno),
                };
                assert_eq!(system_error_errno(code), errno);
            }
        }
    }
}

#[allow(non_upper_case_globals)]
#[cfg(ossl101)]
#[pymodule(sub, name = "_ssl")]
mod ossl101 {
    #[pyattr]
    use openssl_sys::{
        SSL_OP_NO_COMPRESSION as OP_NO_COMPRESSION, SSL_OP_NO_TLSv1_1 as OP_NO_TLSv1_1,
        SSL_OP_NO_TLSv1_2 as OP_NO_TLSv1_2,
    };
}

#[allow(non_upper_case_globals)]
#[cfg(ossl111)]
#[pymodule(sub, name = "_ssl")]
mod ossl111 {
    #[pyattr]
    use openssl_sys::SSL_OP_NO_TLSv1_3 as OP_NO_TLSv1_3;
}

#[cfg(windows)]
#[pymodule(sub, name = "_ssl")]
mod windows {
    use crate::{
        common::ascii,
        vm::{
            PyObjectRef, PyPayload, PyResult, VirtualMachine,
            builtins::{PyFrozenSet, PyStrRef},
            convert::ToPyException,
        },
    };

    #[pyfunction]
    fn enum_certificates(store_name: PyStrRef, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let certs = rustpython_host_env::cert_store::enum_certificates(store_name.as_str());
        let certs = certs.entries.into_iter().map(|c| {
            let cert = vm.ctx.new_bytes(c.der);
            let enc_type = match c.encoding {
                rustpython_host_env::cert_store::EncodingType::X509Asn => {
                    vm.new_pyobj(ascii!("x509_asn"))
                }
                rustpython_host_env::cert_store::EncodingType::Pkcs7Asn => {
                    vm.new_pyobj(ascii!("pkcs_7_asn"))
                }
                rustpython_host_env::cert_store::EncodingType::Other(other) => vm.new_pyobj(other),
            };
            let usage: PyObjectRef = match c.valid_uses.map_err(|e| e.to_pyexception(vm))? {
                rustpython_host_env::cert_store::CertificateUses::All => {
                    vm.ctx.new_bool(true).into()
                }
                rustpython_host_env::cert_store::CertificateUses::Oids(oids) => {
                    PyFrozenSet::from_iter(
                        vm,
                        oids.into_iter().map(|oid| vm.ctx.new_str(oid).into()),
                    )?
                    .into_ref(&vm.ctx)
                    .into()
                }
            };
            Ok(vm.new_tuple((cert, enc_type, usage)).into())
        });
        let certs: Vec<PyObjectRef> = certs.collect::<PyResult<Vec<_>>>()?;
        Ok(certs)
    }
}

mod bio {
    //! based off rust-openssl's private `bio` module

    use core::ffi::c_int;
    use openssl::error::ErrorStack;
    use openssl_sys as sys;
    use std::marker::PhantomData;

    pub(super) struct MemBioSlice<'a>(*mut sys::BIO, PhantomData<&'a [u8]>);

    impl Drop for MemBioSlice<'_> {
        fn drop(&mut self) {
            unsafe {
                sys::BIO_free_all(self.0);
            }
        }
    }

    impl<'a> MemBioSlice<'a> {
        pub(super) fn new(buf: &'a [u8]) -> Result<MemBioSlice<'a>, ErrorStack> {
            openssl::init();

            assert!(buf.len() <= c_int::MAX as usize);
            let bio = unsafe { sys::BIO_new_mem_buf(buf.as_ptr() as *const _, buf.len() as c_int) };
            if bio.is_null() {
                return Err(ErrorStack::get());
            }

            Ok(MemBioSlice(bio, PhantomData))
        }

        pub(super) fn as_ptr(&self) -> *mut sys::BIO {
            self.0
        }
    }
}
