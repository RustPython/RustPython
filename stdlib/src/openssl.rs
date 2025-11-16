// spell-checker:disable

mod cert;

// Conditional compilation for OpenSSL version-specific error codes
cfg_if::cfg_if! {
    if #[cfg(ossl310)] {
        // OpenSSL 3.1.0+
        mod ssl_data_31;
        use ssl_data_31 as ssl_data;
    } else if #[cfg(ossl300)] {
        // OpenSSL 3.0.0+
        mod ssl_data_300;
        use ssl_data_300 as ssl_data;
    } else {
        // OpenSSL 1.1.1+ (fallback)
        mod ssl_data_111;
        use ssl_data_111 as ssl_data;
    }
}

use crate::vm::{PyRef, VirtualMachine, builtins::PyModule};
use openssl_probe::ProbeResult;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    // if openssl is vendored, it doesn't know the locations
    // of system certificates - cache the probe result now.
    #[cfg(openssl_vendored)]
    LazyLock::force(&PROBE);
    _ssl::make_module(vm)
}

// define our own copy of ProbeResult so we can handle the vendor case
// easily, without having to have a bunch of cfgs
cfg_if::cfg_if! {
    if #[cfg(openssl_vendored)] {
        use std::sync::LazyLock;
        static PROBE: LazyLock<ProbeResult> = LazyLock::new(openssl_probe::probe);
        fn probe() -> &'static ProbeResult { &PROBE }
    } else {
        fn probe() -> &'static ProbeResult {
            &ProbeResult { cert_file: None, cert_dir: None }
        }
    }
}

#[allow(non_upper_case_globals)]
#[pymodule(with(cert::ssl_cert, ossl101, ossl111, windows))]
mod _ssl {
    use super::{bio, probe};
    use crate::{
        common::lock::{
            PyMappedRwLockReadGuard, PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard,
        },
        socket::{self, PySocket},
        vm::{
            AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
            builtins::{
                PyBaseExceptionRef, PyBytesRef, PyListRef, PyOSError, PyStrRef, PyTypeRef, PyWeak,
            },
            class_or_notimplemented,
            convert::ToPyException,
            exceptions,
            function::{
                ArgBytesLike, ArgCallable, ArgMemoryBuffer, ArgStrOrBytesLike, Either, FsPath,
                OptionalArg, PyComparisonValue,
            },
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
    use std::{
        ffi::CStr,
        fmt,
        io::{Read, Write},
        path::{Path, PathBuf},
        sync::LazyLock,
        time::Instant,
    };

    // Import certificate types from parent module
    use super::cert::{self, cert_to_certificate, cert_to_py};

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

    // SSL Alert Descriptions (RFC 5246 and extensions)
    // Hybrid approach: use openssl_sys constants where available, hardcode others
    #[pyattr]
    const ALERT_DESCRIPTION_CLOSE_NOTIFY: libc::c_int = 0;
    #[pyattr]
    const ALERT_DESCRIPTION_UNEXPECTED_MESSAGE: libc::c_int = 10;
    #[pyattr]
    const ALERT_DESCRIPTION_BAD_RECORD_MAC: libc::c_int = 20;
    #[pyattr]
    const ALERT_DESCRIPTION_RECORD_OVERFLOW: libc::c_int = 22;
    #[pyattr]
    const ALERT_DESCRIPTION_DECOMPRESSION_FAILURE: libc::c_int = 30;
    #[pyattr]
    const ALERT_DESCRIPTION_HANDSHAKE_FAILURE: libc::c_int = 40;
    #[pyattr]
    const ALERT_DESCRIPTION_BAD_CERTIFICATE: libc::c_int = 42;
    #[pyattr]
    const ALERT_DESCRIPTION_UNSUPPORTED_CERTIFICATE: libc::c_int = 43;
    #[pyattr]
    const ALERT_DESCRIPTION_CERTIFICATE_REVOKED: libc::c_int = 44;
    #[pyattr]
    const ALERT_DESCRIPTION_CERTIFICATE_EXPIRED: libc::c_int = 45;
    #[pyattr]
    const ALERT_DESCRIPTION_CERTIFICATE_UNKNOWN: libc::c_int = 46;
    #[pyattr]
    const ALERT_DESCRIPTION_ILLEGAL_PARAMETER: libc::c_int = SSL_AD_ILLEGAL_PARAMETER;
    #[pyattr]
    const ALERT_DESCRIPTION_UNKNOWN_CA: libc::c_int = 48;
    #[pyattr]
    const ALERT_DESCRIPTION_ACCESS_DENIED: libc::c_int = 49;
    #[pyattr]
    const ALERT_DESCRIPTION_DECODE_ERROR: libc::c_int = SSL_AD_DECODE_ERROR;
    #[pyattr]
    const ALERT_DESCRIPTION_DECRYPT_ERROR: libc::c_int = 51;
    #[pyattr]
    const ALERT_DESCRIPTION_PROTOCOL_VERSION: libc::c_int = 70;
    #[pyattr]
    const ALERT_DESCRIPTION_INSUFFICIENT_SECURITY: libc::c_int = 71;
    #[pyattr]
    const ALERT_DESCRIPTION_INTERNAL_ERROR: libc::c_int = 80;
    #[pyattr]
    const ALERT_DESCRIPTION_USER_CANCELLED: libc::c_int = 90;
    #[pyattr]
    const ALERT_DESCRIPTION_NO_RENEGOTIATION: libc::c_int = 100;
    #[pyattr]
    const ALERT_DESCRIPTION_UNSUPPORTED_EXTENSION: libc::c_int = 110;
    #[pyattr]
    const ALERT_DESCRIPTION_CERTIFICATE_UNOBTAINABLE: libc::c_int = 111;
    #[pyattr]
    const ALERT_DESCRIPTION_UNRECOGNIZED_NAME: libc::c_int = SSL_AD_UNRECOGNIZED_NAME;
    #[pyattr]
    const ALERT_DESCRIPTION_BAD_CERTIFICATE_STATUS_RESPONSE: libc::c_int = 113;
    #[pyattr]
    const ALERT_DESCRIPTION_BAD_CERTIFICATE_HASH_VALUE: libc::c_int = 114;
    #[pyattr]
    const ALERT_DESCRIPTION_UNKNOWN_PSK_IDENTITY: libc::c_int = 115;

    // CRL verification constants
    #[pyattr]
    const VERIFY_CRL_CHECK_CHAIN: libc::c_ulong =
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
    const OP_ALL: libc::c_ulong = (sys::SSL_OP_ALL & !sys::SSL_OP_DONT_INSERT_EMPTY_FRAGMENTS) as _;
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
    const SSL_ERROR_EOF: u32 = 8; // custom for python
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
    const HAS_PSK: bool = true;

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
    const SSL_VERIFY_POST_HANDSHAKE: libc::c_int = 0x20;

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

    // SSL Exception Types

    /// An error occurred in the SSL implementation.
    #[pyattr]
    #[pyexception(name = "SSLError", base = PyOSError)]
    #[derive(Debug)]
    pub struct PySslError {}

    #[pyexception]
    impl PySslError {
        // Returns strerror attribute if available, otherwise str(args)
        #[pymethod]
        fn __str__(exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            // Try to get strerror attribute first (OSError compatibility)
            if let Ok(strerror) = exc.as_object().get_attr("strerror", vm)
                && !vm.is_none(&strerror)
            {
                return strerror.str(vm);
            }

            // Otherwise return str(args)
            exc.args().as_object().str(vm)
        }
    }

    /// A certificate could not be verified.
    #[pyattr]
    #[pyexception(name = "SSLCertVerificationError", base = PySslError)]
    #[derive(Debug)]
    pub struct PySslCertVerificationError {}

    #[pyexception]
    impl PySslCertVerificationError {}

    /// SSL/TLS session closed cleanly.
    #[pyattr]
    #[pyexception(name = "SSLZeroReturnError", base = PySslError)]
    #[derive(Debug)]
    pub struct PySslZeroReturnError {}

    #[pyexception]
    impl PySslZeroReturnError {}

    /// Non-blocking SSL socket needs to read more data.
    #[pyattr]
    #[pyexception(name = "SSLWantReadError", base = PySslError)]
    #[derive(Debug)]
    pub struct PySslWantReadError {}

    #[pyexception]
    impl PySslWantReadError {}

    /// Non-blocking SSL socket needs to write more data.
    #[pyattr]
    #[pyexception(name = "SSLWantWriteError", base = PySslError)]
    #[derive(Debug)]
    pub struct PySslWantWriteError {}

    #[pyexception]
    impl PySslWantWriteError {}

    /// System error when attempting SSL operation.
    #[pyattr]
    #[pyexception(name = "SSLSyscallError", base = PySslError)]
    #[derive(Debug)]
    pub struct PySslSyscallError {}

    #[pyexception]
    impl PySslSyscallError {}

    /// SSL/TLS connection terminated abruptly.
    #[pyattr]
    #[pyexception(name = "SSLEOFError", base = PySslError)]
    #[derive(Debug)]
    pub struct PySslEOFError {}

    #[pyexception]
    impl PySslEOFError {}

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

    type PyNid = (libc::c_int, String, String, Option<String>);
    fn obj2py(obj: &Asn1ObjectRef, vm: &VirtualMachine) -> PyResult<PyNid> {
        let nid = obj.nid();
        let short_name = nid
            .short_name()
            .map_err(|_| vm.new_value_error("NID has no short name".to_owned()))?
            .to_owned();
        let long_name = nid
            .long_name()
            .map_err(|_| vm.new_value_error("NID has no long name".to_owned()))?
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
        #[pyarg(any, default = false)]
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
    fn nid2obj(nid: libc::c_int, vm: &VirtualMachine) -> PyResult<PyNid> {
        _nid2obj(Nid::from_raw(nid))
            .as_deref()
            .ok_or_else(|| vm.new_value_error(format!("unknown NID {nid}")))
            .and_then(|obj| obj2py(obj, vm))
    }

    // Lazily compute and cache cert file/dir paths
    static CERT_PATHS: LazyLock<(PathBuf, PathBuf)> = LazyLock::new(|| {
        fn path_from_cstr(c: &CStr) -> PathBuf {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                std::ffi::OsStr::from_bytes(c.to_bytes()).into()
            }
            #[cfg(windows)]
            {
                // Use lossy conversion for potential non-UTF8
                PathBuf::from(c.to_string_lossy().as_ref())
            }
        }

        let probe = probe();
        let cert_file = probe
            .cert_file
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                path_from_cstr(unsafe { CStr::from_ptr(sys::X509_get_default_cert_file()) })
            });
        let cert_dir = probe
            .cert_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                path_from_cstr(unsafe { CStr::from_ptr(sys::X509_get_default_cert_dir()) })
            });
        (cert_file, cert_dir)
    });

    fn get_cert_file_dir() -> (&'static Path, &'static Path) {
        let (cert_file, cert_dir) = &*CERT_PATHS;
        (cert_file.as_path(), cert_dir.as_path())
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
        let cert_file = OsPath::new_str(cert_file).filename(vm);
        let cert_dir = OsPath::new_str(cert_dir).filename(vm);
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
            for buf in b.chunks(libc::c_int::MAX as usize) {
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
        let mut buf = vec![0; n as usize];
        openssl::rand::rand_bytes(&mut buf).map_err(|e| convert_openssl_error(vm, e))?;
        Ok(buf)
    }

    // Callback data stored in SSL context for SNI
    struct SniCallbackData {
        ssl_context: PyRef<PySslContext>,
        vm_ptr: *const VirtualMachine,
    }

    impl Drop for SniCallbackData {
        fn drop(&mut self) {
            // PyRef will handle reference counting
        }
    }

    // Get or create an ex_data index for SNI callback data
    fn get_sni_ex_data_index() -> libc::c_int {
        use std::sync::LazyLock;
        static SNI_EX_DATA_IDX: LazyLock<libc::c_int> = LazyLock::new(|| unsafe {
            sys::SSL_get_ex_new_index(
                0,
                std::ptr::null_mut(),
                None,
                None,
                Some(sni_callback_data_free),
            )
        });
        *SNI_EX_DATA_IDX
    }

    // Free function for callback data
    unsafe extern "C" fn sni_callback_data_free(
        _parent: *mut libc::c_void,
        ptr: *mut libc::c_void,
        _ad: *mut sys::CRYPTO_EX_DATA,
        _idx: libc::c_int,
        _argl: libc::c_long,
        _argp: *mut libc::c_void,
    ) {
        if !ptr.is_null() {
            unsafe {
                let _ = Box::from_raw(ptr as *mut SniCallbackData);
            }
        }
    }

    // SNI callback function called by OpenSSL
    unsafe extern "C" fn _servername_callback(
        ssl_ptr: *mut sys::SSL,
        al: *mut libc::c_int,
        arg: *mut libc::c_void,
    ) -> libc::c_int {
        const SSL_TLSEXT_ERR_OK: libc::c_int = 0;
        const SSL_TLSEXT_ERR_ALERT_FATAL: libc::c_int = 2;
        const SSL_AD_INTERNAL_ERROR: libc::c_int = 80;
        const TLSEXT_NAMETYPE_host_name: libc::c_int = 0;

        if arg.is_null() {
            return SSL_TLSEXT_ERR_OK;
        }

        unsafe {
            let ctx = &*(arg as *const PySslContext);

            // Get the callback
            let callback_opt = ctx.sni_callback.lock().clone();
            let Some(callback) = callback_opt else {
                return SSL_TLSEXT_ERR_OK;
            };

            // Get callback data from SSL ex_data
            let idx = get_sni_ex_data_index();
            let data_ptr = sys::SSL_get_ex_data(ssl_ptr, idx);
            if data_ptr.is_null() {
                return SSL_TLSEXT_ERR_ALERT_FATAL;
            }

            let callback_data = &*(data_ptr as *const SniCallbackData);

            // SAFETY: vm_ptr is stored during wrap_socket and is valid for the lifetime
            // of the SSL connection. The handshake happens synchronously in the same thread.
            let vm = &*callback_data.vm_ptr;

            // Get server name
            let servername = sys::SSL_get_servername(ssl_ptr, TLSEXT_NAMETYPE_host_name);
            let server_name_arg = if servername.is_null() {
                vm.ctx.none()
            } else {
                let name_cstr = std::ffi::CStr::from_ptr(servername);
                match name_cstr.to_str() {
                    Ok(name_str) => vm.ctx.new_str(name_str).into(),
                    Err(_) => vm.ctx.none(),
                }
            };

            // Get SSL socket from SSL ex_data (stored as PySslSocket pointer)
            let ssl_socket_ptr = sys::SSL_get_ex_data(ssl_ptr, 0); // Index 0 for SSL socket
            let ssl_socket_obj = if !ssl_socket_ptr.is_null() {
                let ssl_socket = &*(ssl_socket_ptr as *const PySslSocket);
                // Try to get owner first
                ssl_socket
                    .owner
                    .read()
                    .as_ref()
                    .and_then(|weak| weak.upgrade())
                    .unwrap_or_else(|| vm.ctx.none())
            } else {
                vm.ctx.none()
            };

            // Call the Python callback
            match callback.call(
                (
                    ssl_socket_obj,
                    server_name_arg,
                    callback_data.ssl_context.to_owned(),
                ),
                vm,
            ) {
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
                    *al = SSL_AD_INTERNAL_ERROR;
                    SSL_TLSEXT_ERR_ALERT_FATAL
                }
            }
        }
    }

    // Message callback function called by OpenSSL
    // Based on CPython's _PySSL_msg_callback in Modules/_ssl/debughelpers.c
    unsafe extern "C" fn _msg_callback(
        write_p: libc::c_int,
        version: libc::c_int,
        content_type: libc::c_int,
        buf: *const libc::c_void,
        len: usize,
        ssl_ptr: *mut sys::SSL,
        _arg: *mut libc::c_void,
    ) {
        if ssl_ptr.is_null() {
            return;
        }

        unsafe {
            // Get SSL socket from SSL_get_app_data (index 0)
            let ssl_socket_ptr = sys::SSL_get_ex_data(ssl_ptr, 0);
            if ssl_socket_ptr.is_null() {
                return;
            }

            let ssl_socket = &*(ssl_socket_ptr as *const PySslSocket);

            // Get the callback from the context
            let callback_opt = ssl_socket.ctx.read().msg_callback.lock().clone();
            let Some(callback) = callback_opt else {
                return;
            };

            // Get callback data from SSL ex_data (for VM)
            let idx = get_sni_ex_data_index();
            let data_ptr = sys::SSL_get_ex_data(ssl_ptr, idx);
            if data_ptr.is_null() {
                return;
            }

            let callback_data = &*(data_ptr as *const SniCallbackData);
            let vm = &*callback_data.vm_ptr;

            // Get SSL socket owner object
            let ssl_socket_obj = ssl_socket
                .owner
                .read()
                .as_ref()
                .and_then(|weak| weak.upgrade())
                .unwrap_or_else(|| vm.ctx.none());

            // Create the message bytes
            let buf_slice = std::slice::from_raw_parts(buf as *const u8, len);
            let msg_bytes = vm.ctx.new_bytes(buf_slice.to_vec());

            // Determine direction string
            let direction_str = if write_p != 0 { "write" } else { "read" };

            // Call the Python callback
            // Signature: callback(conn, direction, version, content_type, msg_type, data)
            // For simplicity, we'll pass msg_type as 0 (would need more parsing to get the actual type)
            match callback.call(
                (
                    ssl_socket_obj,
                    vm.ctx.new_str(direction_str),
                    vm.ctx.new_int(version),
                    vm.ctx.new_int(content_type),
                    vm.ctx.new_int(0), // msg_type - would need parsing
                    msg_bytes,
                ),
                vm,
            ) {
                Ok(_) => {}
                Err(exc) => {
                    // Log the exception but don't propagate it
                    vm.run_unraisable(exc, None, vm.ctx.none());
                }
            }
        }
    }

    #[pyfunction(name = "RAND_pseudo_bytes")]
    fn rand_pseudo_bytes(n: i32, vm: &VirtualMachine) -> PyResult<(Vec<u8>, bool)> {
        if n < 0 {
            return Err(vm.new_value_error("num must be positive"));
        }
        let mut buf = vec![0; n as usize];
        let ret = unsafe { sys::RAND_bytes(buf.as_mut_ptr(), n) };
        match ret {
            0 | 1 => Ok((buf, ret == 1)),
            _ => Err(convert_openssl_error(vm, ErrorStack::get())),
        }
    }

    #[pyattr]
    #[pyclass(module = "ssl", name = "_SSLContext")]
    #[derive(PyPayload)]
    struct PySslContext {
        ctx: PyRwLock<SslContextBuilder>,
        check_hostname: AtomicCell<bool>,
        protocol: SslVersion,
        post_handshake_auth: PyMutex<bool>,
        sni_callback: PyMutex<Option<PyObjectRef>>,
        msg_callback: PyMutex<Option<PyObjectRef>>,
    }

    impl fmt::Debug for PySslContext {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("_SSLContext")
        }
    }

    fn builder_as_ctx(x: &SslContextBuilder) -> &ssl::SslContextRef {
        unsafe { ssl::SslContextRef::from_ptr(x.as_ptr()) }
    }

    impl Constructor for PySslContext {
        type Args = i32;

        fn py_new(cls: PyTypeRef, proto_version: Self::Args, vm: &VirtualMachine) -> PyResult {
            let proto = SslVersion::try_from(proto_version)
                .map_err(|_| vm.new_value_error("invalid protocol version"))?;
            let method = match proto {
                // SslVersion::Ssl3 => unsafe { ssl::SslMethod::from_ptr(sys::SSLv3_method()) },
                SslVersion::Tls => ssl::SslMethod::tls(),
                SslVersion::Tls1 => ssl::SslMethod::tls(),
                SslVersion::Tls1_1 => ssl::SslMethod::tls(),
                SslVersion::Tls1_2 => ssl::SslMethod::tls(),
                SslVersion::TlsClient => ssl::SslMethod::tls_client(),
                SslVersion::TlsServer => ssl::SslMethod::tls_server(),
                _ => return Err(vm.new_value_error("invalid protocol version")),
            };
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

            PySslContext {
                ctx: PyRwLock::new(builder),
                check_hostname: AtomicCell::new(check_hostname),
                protocol: proto,
                post_handshake_auth: PyMutex::new(false),
                sni_callback: PyMutex::new(None),
                msg_callback: PyMutex::new(None),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(flags(BASETYPE, IMMUTABLETYPE), with(Constructor))]
    impl PySslContext {
        fn builder(&self) -> PyRwLockWriteGuard<'_, SslContextBuilder> {
            self.ctx.write()
        }
        fn ctx(&self) -> PyMappedRwLockReadGuard<'_, ssl::SslContextRef> {
            PyRwLockReadGuard::map(self.ctx.read(), builder_as_ctx)
        }

        #[pygetset]
        fn post_handshake_auth(&self) -> bool {
            *self.post_handshake_auth.lock()
        }
        #[pygetset(setter)]
        fn set_post_handshake_auth(
            &self,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let value = value.ok_or_else(|| vm.new_attribute_error("cannot delete attribute"))?;
            *self.post_handshake_auth.lock() = value.is_true(vm)?;
            Ok(())
        }

        #[cfg(ossl110)]
        #[pygetset]
        fn security_level(&self) -> i32 {
            unsafe { SSL_CTX_get_security_level(self.ctx().as_ptr()) }
        }

        #[pymethod]
        fn set_ciphers(&self, cipherlist: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
            let ciphers = cipherlist.as_str();
            if ciphers.contains('\0') {
                return Err(exceptions::cstring_error(vm));
            }
            self.builder().set_cipher_list(ciphers).map_err(|_| {
                vm.new_exception_msg(
                    PySslError::class(&vm.ctx).to_owned(),
                    "No cipher can be selected.".to_owned(),
                )
            })
        }

        #[pymethod]
        fn get_ciphers(&self, vm: &VirtualMachine) -> PyResult<PyListRef> {
            let ctx = self.ctx();
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
            &self,
            name: Either<PyStrRef, ArgBytesLike>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            use openssl::ec::{EcGroup, EcKey};

            // Convert name to CString, supporting both str and bytes
            let name_cstr = match name {
                Either::A(s) => {
                    if s.as_str().contains('\0') {
                        return Err(exceptions::cstring_error(vm));
                    }
                    s.to_cstring(vm)?
                }
                Either::B(b) => std::ffi::CString::new(b.borrow_buf().to_vec())
                    .map_err(|_| exceptions::cstring_error(vm))?,
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
            self.builder()
                .set_tmp_ecdh(&key)
                .map_err(|e| convert_openssl_error(vm, e))
        }

        #[pygetset]
        fn options(&self) -> libc::c_ulong {
            self.ctx.read().options().bits() as _
        }
        #[pygetset(setter)]
        fn set_options(&self, opts: libc::c_ulong) {
            self.builder()
                .set_options(SslOptions::from_bits_truncate(opts as _));
        }
        #[pygetset]
        fn protocol(&self) -> i32 {
            self.protocol as i32
        }
        #[pygetset]
        fn verify_mode(&self) -> i32 {
            let mode = self.ctx().verify_mode();
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
        fn set_verify_mode(&self, cert: i32, vm: &VirtualMachine) -> PyResult<()> {
            let mut ctx = self.builder();
            let cert_req = CertRequirements::try_from(cert)
                .map_err(|_| vm.new_value_error("invalid value for verify_mode"))?;
            let mode = match cert_req {
                CertRequirements::None if self.check_hostname.load() => {
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
        fn verify_flags(&self) -> libc::c_ulong {
            unsafe {
                let ctx_ptr = self.ctx().as_ptr();
                let param = sys::SSL_CTX_get0_param(ctx_ptr);
                sys::X509_VERIFY_PARAM_get_flags(param)
            }
        }
        #[pygetset(setter)]
        fn set_verify_flags(&self, new_flags: libc::c_ulong, vm: &VirtualMachine) -> PyResult<()> {
            unsafe {
                let ctx_ptr = self.ctx().as_ptr();
                let param = sys::SSL_CTX_get0_param(ctx_ptr);
                let flags = sys::X509_VERIFY_PARAM_get_flags(param);
                let clear = flags & !new_flags;
                let set = !flags & new_flags;

                if clear != 0 && sys::X509_VERIFY_PARAM_clear_flags(param, clear) == 0 {
                    return Err(vm.new_exception_msg(
                        PySslError::class(&vm.ctx).to_owned(),
                        "Failed to clear verify flags".to_owned(),
                    ));
                }
                if set != 0 && sys::X509_VERIFY_PARAM_set_flags(param, set) == 0 {
                    return Err(vm.new_exception_msg(
                        PySslError::class(&vm.ctx).to_owned(),
                        "Failed to set verify flags".to_owned(),
                    ));
                }
                Ok(())
            }
        }
        #[pygetset]
        fn check_hostname(&self) -> bool {
            self.check_hostname.load()
        }
        #[pygetset(setter)]
        fn set_check_hostname(&self, ch: bool) {
            let mut ctx = self.builder();
            if ch && builder_as_ctx(&ctx).verify_mode() == SslVerifyMode::NONE {
                ctx.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
            }
            self.check_hostname.store(ch);
        }

        // PY_PROTO_MINIMUM_SUPPORTED = -2, PY_PROTO_MAXIMUM_SUPPORTED = -1
        #[pygetset]
        fn minimum_version(&self) -> i32 {
            let ctx = self.ctx();
            let version = unsafe { sys::SSL_CTX_get_min_proto_version(ctx.as_ptr()) };
            if version == 0 {
                -2 // PY_PROTO_MINIMUM_SUPPORTED
            } else {
                version
            }
        }
        #[pygetset(setter)]
        fn set_minimum_version(&self, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            // Handle special values
            let proto_version = match value {
                -2 => {
                    // PY_PROTO_MINIMUM_SUPPORTED -> use minimum available (TLS 1.2)
                    sys::TLS1_2_VERSION
                }
                -1 => {
                    // PY_PROTO_MAXIMUM_SUPPORTED -> use maximum available
                    // For max on min_proto_version, we use the newest available
                    sys::TLS1_3_VERSION
                }
                _ => value,
            };

            let ctx = self.builder();
            let result = unsafe { sys::SSL_CTX_set_min_proto_version(ctx.as_ptr(), proto_version) };
            if result == 0 {
                return Err(vm.new_value_error("invalid protocol version"));
            }
            Ok(())
        }

        #[pygetset]
        fn maximum_version(&self) -> i32 {
            let ctx = self.ctx();
            let version = unsafe { sys::SSL_CTX_get_max_proto_version(ctx.as_ptr()) };
            if version == 0 {
                -1 // PY_PROTO_MAXIMUM_SUPPORTED
            } else {
                version
            }
        }
        #[pygetset(setter)]
        fn set_maximum_version(&self, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            // Handle special values
            let proto_version = match value {
                -1 => {
                    // PY_PROTO_MAXIMUM_SUPPORTED -> use 0 for OpenSSL (means no limit)
                    0
                }
                -2 => {
                    // PY_PROTO_MINIMUM_SUPPORTED -> use minimum available (TLS 1.2)
                    sys::TLS1_2_VERSION
                }
                _ => value,
            };

            let ctx = self.builder();
            let result = unsafe { sys::SSL_CTX_set_max_proto_version(ctx.as_ptr(), proto_version) };
            if result == 0 {
                return Err(vm.new_value_error("invalid protocol version"));
            }
            Ok(())
        }

        #[pygetset]
        fn num_tickets(&self, _vm: &VirtualMachine) -> PyResult<usize> {
            // Only supported for TLS 1.3
            #[cfg(ossl110)]
            {
                let ctx = self.ctx();
                let num = unsafe { sys::SSL_CTX_get_num_tickets(ctx.as_ptr()) };
                Ok(num)
            }
            #[cfg(not(ossl110))]
            {
                Ok(0)
            }
        }
        #[pygetset(setter)]
        fn set_num_tickets(&self, value: isize, vm: &VirtualMachine) -> PyResult<()> {
            // Check for negative values
            if value < 0 {
                return Err(
                    vm.new_value_error("num_tickets must be a non-negative integer".to_owned())
                );
            }

            // Check that this is a server context
            if self.protocol != SslVersion::TlsServer {
                return Err(vm.new_value_error("SSLContext is not a server context.".to_owned()));
            }

            #[cfg(ossl110)]
            {
                let ctx = self.builder();
                let result = unsafe { sys::SSL_CTX_set_num_tickets(ctx.as_ptr(), value as usize) };
                if result != 1 {
                    return Err(vm.new_value_error("failed to set num tickets."));
                }
                Ok(())
            }
            #[cfg(not(ossl110))]
            {
                let _ = (value, vm);
                Ok(())
            }
        }

        #[pymethod]
        fn set_default_verify_paths(&self, vm: &VirtualMachine) -> PyResult<()> {
            cfg_if::cfg_if! {
                if #[cfg(openssl_vendored)] {
                    let (cert_file, cert_dir) = get_cert_file_dir();
                    self.builder()
                        .load_verify_locations(Some(cert_file), Some(cert_dir))
                        .map_err(|e| convert_openssl_error(vm, e))
                } else {
                    self.builder()
                        .set_default_verify_paths()
                        .map_err(|e| convert_openssl_error(vm, e))
                }
            }
        }

        #[pymethod]
        fn _set_alpn_protocols(&self, protos: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
            #[cfg(ossl102)]
            {
                let mut ctx = self.builder();
                let server = protos.with_ref(|pbuf| {
                    if pbuf.len() > libc::c_uint::MAX as usize {
                        return Err(vm.new_overflow_error(format!(
                            "protocols longer than {} bytes",
                            libc::c_uint::MAX
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
                    Ok(&client[pos..proto.len()])
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
        fn load_verify_locations(
            &self,
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

            let mut ctx = self.builder();

            // validate cadata type and load cadata
            if let Some(cadata) = args.cadata {
                let certs = match cadata {
                    Either::A(s) => {
                        if !s.is_ascii() {
                            return Err(invalid_cadata(vm));
                        }
                        X509::stack_from_pem(s.as_bytes())
                    }
                    Either::B(b) => b.with_ref(x509_stack_from_der),
                };
                let certs = certs.map_err(|e| convert_openssl_error(vm, e))?;
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
                ctx.load_verify_locations(cafile_path.as_deref(), capath_path.as_deref())
                    .map_err(|e| convert_openssl_error(vm, e))?;
            }

            Ok(())
        }

        #[pymethod]
        fn get_ca_certs(
            &self,
            binary_form: OptionalArg<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<PyObjectRef>> {
            let binary_form = binary_form.unwrap_or(false);
            let ctx = self.ctx();
            #[cfg(ossl300)]
            let certs = ctx.cert_store().all_certificates();
            #[cfg(not(ossl300))]
            let certs = ctx.cert_store().objects().iter().filter_map(|x| x.x509());

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
        fn cert_store_stats(&self, vm: &VirtualMachine) -> PyResult {
            let ctx = self.ctx();
            let store_ptr = unsafe { sys::SSL_CTX_get_cert_store(ctx.as_ptr()) };

            if store_ptr.is_null() {
                return Err(vm.new_memory_error("failed to get cert store".to_owned()));
            }

            let objs_ptr = unsafe { sys::X509_STORE_get0_objects(store_ptr) };
            if objs_ptr.is_null() {
                return Err(vm.new_memory_error("failed to query cert store".to_owned()));
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
                            if !x509_ptr.is_null() && X509_check_ca(x509_ptr) == 1 {
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
        fn session_stats(&self, vm: &VirtualMachine) -> PyResult {
            let ctx = self.ctx();
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
        fn load_dh_params(&self, filepath: FsPath, vm: &VirtualMachine) -> PyResult<()> {
            let path = filepath.to_path_buf(vm)?;

            // Open the file using fopen (cross-platform)
            let fp =
                rustpython_common::fileutils::fopen(path.as_path(), "rb").map_err(|e| {
                    match e.kind() {
                        std::io::ErrorKind::NotFound => vm.new_exception_msg(
                            vm.ctx.exceptions.file_not_found_error.to_owned(),
                            e.to_string(),
                        ),
                        _ => vm.new_os_error(e.to_string()),
                    }
                })?;

            // Read DH parameters
            let dh = unsafe {
                PEM_read_DHparams(
                    fp,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            unsafe {
                libc::fclose(fp);
            }

            if dh.is_null() {
                return Err(convert_openssl_error(vm, ErrorStack::get()));
            }

            // Set temporary DH parameters
            let ctx = self.builder();
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
        fn sni_callback(&self) -> Option<PyObjectRef> {
            self.sni_callback.lock().clone()
        }

        #[pygetset(setter)]
        fn set_sni_callback(
            &self,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // Check if this is a server context
            if self.protocol == SslVersion::TlsClient {
                return Err(vm.new_value_error(
                    "sni_callback cannot be set on TLS_CLIENT context".to_owned(),
                ));
            }

            let mut callback_guard = self.sni_callback.lock();

            if let Some(callback_obj) = value {
                if !vm.is_none(&callback_obj) {
                    // Check if callable
                    if !callback_obj.is_callable() {
                        return Err(vm.new_type_error("not a callable object".to_owned()));
                    }

                    // Set the callback
                    *callback_guard = Some(callback_obj);

                    // Set OpenSSL callback
                    unsafe {
                        sys::SSL_CTX_set_tlsext_servername_callback__fixed_rust(
                            self.ctx().as_ptr(),
                            Some(_servername_callback),
                        );
                        sys::SSL_CTX_set_tlsext_servername_arg(
                            self.ctx().as_ptr(),
                            self as *const _ as *mut _,
                        );
                    }
                } else {
                    // Clear callback
                    *callback_guard = None;
                    unsafe {
                        sys::SSL_CTX_set_tlsext_servername_callback__fixed_rust(
                            self.ctx().as_ptr(),
                            None,
                        );
                    }
                }
            } else {
                // Clear callback
                *callback_guard = None;
                unsafe {
                    sys::SSL_CTX_set_tlsext_servername_callback__fixed_rust(
                        self.ctx().as_ptr(),
                        None,
                    );
                }
            }

            Ok(())
        }

        #[pymethod]
        fn set_servername_callback(
            &self,
            callback: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.set_sni_callback(callback, vm)
        }

        #[pygetset(name = "_msg_callback")]
        fn msg_callback(&self) -> Option<PyObjectRef> {
            self.msg_callback.lock().clone()
        }

        #[pygetset(setter, name = "_msg_callback")]
        fn set_msg_callback(
            &self,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let mut callback_guard = self.msg_callback.lock();

            if let Some(callback_obj) = value {
                if !vm.is_none(&callback_obj) {
                    // Check if callable
                    if !callback_obj.is_callable() {
                        return Err(vm.new_type_error("not a callable object".to_owned()));
                    }

                    // Set the callback
                    *callback_guard = Some(callback_obj);

                    // Set OpenSSL callback
                    unsafe {
                        SSL_CTX_set_msg_callback(self.ctx().as_ptr(), Some(_msg_callback));
                    }
                } else {
                    // Clear callback
                    *callback_guard = None;
                    unsafe {
                        SSL_CTX_set_msg_callback(self.ctx().as_ptr(), None);
                    }
                }
            } else {
                // Clear callback when value is None
                *callback_guard = None;
                unsafe {
                    SSL_CTX_set_msg_callback(self.ctx().as_ptr(), None);
                }
            }

            Ok(())
        }

        #[pymethod]
        fn load_cert_chain(&self, args: LoadCertChainArgs, vm: &VirtualMachine) -> PyResult<()> {
            let LoadCertChainArgs {
                certfile,
                keyfile,
                password,
            } = args;
            // TODO: requires passing a callback to C
            if password.is_some() {
                return Err(vm.new_not_implemented_error("password arg not yet supported"));
            }
            let mut ctx = self.builder();
            let key_path = keyfile.map(|path| path.to_path_buf(vm)).transpose()?;
            let cert_path = certfile.to_path_buf(vm)?;
            ctx.set_certificate_chain_file(&cert_path)
                .and_then(|()| {
                    ctx.set_private_key_file(
                        key_path.as_ref().unwrap_or(&cert_path),
                        ssl::SslFiletype::PEM,
                    )
                })
                .and_then(|()| ctx.check_private_key())
                .map_err(|e| convert_openssl_error(vm, e))
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
                return Err(vm.new_exception_msg(
                    PySslError::class(&vm.ctx).to_owned(),
                    "Cannot create a server socket with a PROTOCOL_TLS_CLIENT context".to_owned(),
                ));
            }
            if !server_side && ctx_ref.protocol == SslVersion::TlsServer {
                return Err(vm.new_exception_msg(
                    PySslError::class(&vm.ctx).to_owned(),
                    "Cannot create a client socket with a PROTOCOL_TLS_SERVER context".to_owned(),
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
                        SID_CTX.len() as libc::c_uint,
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
                let hostname_str = hostname.as_str();
                if hostname_str.is_empty() || hostname_str.starts_with('.') {
                    return Err(vm.new_value_error(
                        "server_hostname cannot be an empty string or start with a leading dot.",
                    ));
                }
                if hostname_str.contains('\0') {
                    return Err(vm.new_value_error("embedded null byte in server_hostname"));
                }
                let ip = hostname_str.parse::<std::net::IpAddr>();
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
                        if (mode & sys::SSL_VERIFY_PEER as libc::c_int) != 0 {
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
            let stream = ssl::SslStream::new(ssl, SocketStream(args.sock.clone()))
                .map_err(|e| convert_openssl_error(vm, e))?;

            let py_ssl_socket = PySslSocket {
                ctx: PyRwLock::new(zelf.clone()),
                connection: PyRwLock::new(SslConnection::Socket(stream)),
                socket_type,
                server_hostname,
                owner: PyRwLock::new(args.owner.map(|o| o.downgrade(None, vm)).transpose()?),
            };

            // Convert to PyRef (heap allocation) to avoid use-after-free
            let py_ref =
                py_ssl_socket.into_ref_with_type(vm, PySslSocket::class(&vm.ctx).to_owned())?;

            // Set SNI callback data if callback is configured
            if zelf.sni_callback.lock().is_some() {
                unsafe {
                    let ssl_ptr = py_ref.connection.read().ssl().as_ptr();

                    // Store callback data in SSL ex_data
                    let callback_data = Box::new(SniCallbackData {
                        ssl_context: zelf.clone(),
                        vm_ptr: vm as *const _,
                    });
                    let idx = get_sni_ex_data_index();
                    sys::SSL_set_ex_data(ssl_ptr, idx, Box::into_raw(callback_data) as *mut _);

                    // Store PyRef pointer (heap-allocated) in ex_data index 0
                    sys::SSL_set_ex_data(ssl_ptr, 0, &*py_ref as *const _ as *mut _);
                }
            }

            // Set session if provided
            if let Some(session) = args.session
                && !vm.is_none(&session)
            {
                py_ref.set_session(session, vm)?;
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
            };

            // Create SslStream with BioStream
            let stream =
                ssl::SslStream::new(ssl, bio_stream).map_err(|e| convert_openssl_error(vm, e))?;

            let py_ssl_socket = PySslSocket {
                ctx: PyRwLock::new(zelf.clone()),
                connection: PyRwLock::new(SslConnection::Bio(stream)),
                socket_type,
                server_hostname,
                owner: PyRwLock::new(args.owner.map(|o| o.downgrade(None, vm)).transpose()?),
            };

            // Convert to PyRef (heap allocation) to avoid use-after-free
            let py_ref =
                py_ssl_socket.into_ref_with_type(vm, PySslSocket::class(&vm.ctx).to_owned())?;

            // Set SNI callback data if callback is configured
            if zelf.sni_callback.lock().is_some() {
                unsafe {
                    let ssl_ptr = py_ref.connection.read().ssl().as_ptr();

                    // Store callback data in SSL ex_data
                    let callback_data = Box::new(SniCallbackData {
                        ssl_context: zelf.clone(),
                        vm_ptr: vm as *const _,
                    });
                    let idx = get_sni_ex_data_index();
                    sys::SSL_set_ex_data(ssl_ptr, idx, Box::into_raw(callback_data) as *mut _);

                    // Store PyRef pointer (heap-allocated) in ex_data index 0
                    sys::SSL_set_ex_data(ssl_ptr, 0, &*py_ref as *const _ as *mut _);
                }
            }

            // Set session if provided
            if let Some(session) = args.session
                && !vm.is_none(&session)
            {
                py_ref.set_session(session, vm)?;
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
        password: Option<Either<PyStrRef, ArgCallable>>,
    }

    // Err is true if the socket is blocking
    type SocketDeadline = Result<Instant, bool>;

    enum SelectRet {
        Nonblocking,
        TimedOut,
        IsBlocking,
        Closed,
        Ok,
    }

    #[derive(Clone, Copy)]
    enum SslNeeds {
        Read,
        Write,
    }

    struct SocketStream(PyRef<PySocket>);

    impl SocketStream {
        fn timeout_deadline(&self) -> SocketDeadline {
            self.0.get_timeout().map(|d| Instant::now() + d)
        }

        fn select(&self, needs: SslNeeds, deadline: &SocketDeadline) -> SelectRet {
            let sock = match self.0.sock_opt() {
                Some(s) => s,
                None => return SelectRet::Closed,
            };
            let deadline = match &deadline {
                Ok(deadline) => match deadline.checked_duration_since(Instant::now()) {
                    Some(deadline) => deadline,
                    None => return SelectRet::TimedOut,
                },
                Err(true) => return SelectRet::IsBlocking,
                Err(false) => return SelectRet::Nonblocking,
            };
            let res = socket::sock_select(
                &sock,
                match needs {
                    SslNeeds::Read => socket::SelectKind::Read,
                    SslNeeds::Write => socket::SelectKind::Write,
                },
                Some(deadline),
            );
            match res {
                Ok(true) => SelectRet::TimedOut,
                _ => SelectRet::Ok,
            }
        }

        fn socket_needs(
            &self,
            err: &ssl::Error,
            deadline: &SocketDeadline,
        ) -> (Option<SslNeeds>, SelectRet) {
            let needs = match err.code() {
                ssl::ErrorCode::WANT_READ => Some(SslNeeds::Read),
                ssl::ErrorCode::WANT_WRITE => Some(SslNeeds::Write),
                _ => None,
            };
            let state = needs.map_or(SelectRet::Ok, |needs| self.select(needs, deadline));
            (needs, state)
        }
    }

    fn socket_closed_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(
            PySslError::class(&vm.ctx).to_owned(),
            "Underlying socket has been closed.".to_owned(),
        )
    }

    // BIO stream wrapper to implement Read/Write traits for MemoryBIO
    struct BioStream {
        inbio: PyRef<PySslMemoryBio>,
        outbio: PyRef<PySslMemoryBio>,
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
                Ok(nbytes as usize)
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            // MemoryBIO doesn't need flushing
            Ok(())
        }
    }

    // Enum to represent different SSL connection modes
    enum SslConnection {
        Socket(ssl::SslStream<SocketStream>),
        Bio(ssl::SslStream<BioStream>),
    }

    impl SslConnection {
        // Get a reference to the SSL object
        fn ssl(&self) -> &ssl::SslRef {
            match self {
                SslConnection::Socket(stream) => stream.ssl(),
                SslConnection::Bio(stream) => stream.ssl(),
            }
        }

        // Get underlying socket stream reference (only for socket mode)
        fn get_ref(&self) -> Option<&SocketStream> {
            match self {
                SslConnection::Socket(stream) => Some(stream.get_ref()),
                SslConnection::Bio(_) => None,
            }
        }

        // Check if this is in BIO mode
        fn is_bio(&self) -> bool {
            matches!(self, SslConnection::Bio(_))
        }

        // Perform SSL handshake
        fn do_handshake(&mut self) -> Result<(), ssl::Error> {
            match self {
                SslConnection::Socket(stream) => stream.do_handshake(),
                SslConnection::Bio(stream) => stream.do_handshake(),
            }
        }

        // Write data to SSL connection
        fn ssl_write(&mut self, buf: &[u8]) -> Result<usize, ssl::Error> {
            match self {
                SslConnection::Socket(stream) => stream.ssl_write(buf),
                SslConnection::Bio(stream) => stream.ssl_write(buf),
            }
        }

        // Read data from SSL connection
        fn ssl_read(&mut self, buf: &mut [u8]) -> Result<usize, ssl::Error> {
            match self {
                SslConnection::Socket(stream) => stream.ssl_read(buf),
                SslConnection::Bio(stream) => stream.ssl_read(buf),
            }
        }

        // Get SSL shutdown state
        fn get_shutdown(&mut self) -> ssl::ShutdownState {
            match self {
                SslConnection::Socket(stream) => stream.get_shutdown(),
                SslConnection::Bio(stream) => stream.get_shutdown(),
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "ssl", name = "_SSLSocket", traverse)]
    #[derive(PyPayload)]
    struct PySslSocket {
        ctx: PyRwLock<PyRef<PySslContext>>,
        #[pytraverse(skip)]
        connection: PyRwLock<SslConnection>,
        #[pytraverse(skip)]
        socket_type: SslServerOrClient,
        server_hostname: Option<PyStrRef>,
        owner: PyRwLock<Option<PyRef<PyWeak>>>,
    }

    impl fmt::Debug for PySslSocket {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("_SSLSocket")
        }
    }

    #[pyclass(flags(IMMUTABLETYPE))]
    impl PySslSocket {
        #[pygetset]
        fn owner(&self) -> Option<PyObjectRef> {
            self.owner.read().as_ref().and_then(|weak| weak.upgrade())
        }
        #[pygetset(setter)]
        fn set_owner(&self, owner: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let mut lock = self.owner.write();
            lock.take();
            *lock = Some(owner.downgrade(None, vm)?);
            Ok(())
        }
        #[pygetset]
        fn server_side(&self) -> bool {
            self.socket_type == SslServerOrClient::Server
        }
        #[pygetset]
        fn context(&self) -> PyRef<PySslContext> {
            self.ctx.read().clone()
        }
        #[pygetset(setter)]
        fn set_context(&self, value: PyRef<PySslContext>, vm: &VirtualMachine) -> PyResult<()> {
            // Update the SSL context in the underlying SSL object
            let stream = self.connection.read();

            // Set the new SSL_CTX on the SSL object
            unsafe {
                let result = SSL_set_SSL_CTX(stream.ssl().as_ptr(), value.ctx().as_ptr());
                if result.is_null() {
                    return Err(vm.new_runtime_error("Failed to set SSL context".to_owned()));
                }
            }

            // Update self.ctx to the new context
            *self.ctx.write() = value;
            Ok(())
        }
        #[pygetset]
        fn server_hostname(&self) -> Option<PyStrRef> {
            self.server_hostname.clone()
        }

        #[pymethod]
        fn getpeercert(
            &self,
            binary: OptionalArg<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let binary = binary.unwrap_or(false);
            let stream = self.connection.read();
            if !stream.ssl().is_init_finished() {
                return Err(vm.new_value_error("handshake not done yet"));
            }

            let peer_cert = stream.ssl().peer_certificate();
            let Some(cert) = peer_cert else {
                return Ok(None);
            };

            if binary {
                // Return DER-encoded certificate
                cert_to_py(vm, &cert, true).map(Some)
            } else {
                // Check verify_mode
                unsafe {
                    let ssl_ctx = sys::SSL_get_SSL_CTX(stream.ssl().as_ptr());
                    let verify_mode = sys::SSL_CTX_get_verify_mode(ssl_ctx);
                    if (verify_mode & sys::SSL_VERIFY_PEER as libc::c_int) == 0 {
                        // Return empty dict when SSL_VERIFY_PEER is not set
                        Ok(Some(vm.ctx.new_dict().into()))
                    } else {
                        // Return decoded certificate
                        cert_to_py(vm, &cert, false).map(Some)
                    }
                }
            }
        }

        #[pymethod]
        fn get_unverified_chain(&self, vm: &VirtualMachine) -> PyResult<Option<PyListRef>> {
            let stream = self.connection.read();
            let Some(chain) = stream.ssl().peer_cert_chain() else {
                return Ok(None);
            };

            // Return Certificate objects
            let certs: Vec<PyObjectRef> = chain
                .iter()
                .map(|cert| unsafe {
                    sys::X509_up_ref(cert.as_ptr());
                    let owned = X509::from_ptr(cert.as_ptr());
                    cert_to_certificate(vm, owned)
                })
                .collect::<PyResult<_>>()?;
            Ok(Some(vm.ctx.new_list(certs)))
        }

        #[pymethod]
        fn get_verified_chain(&self, vm: &VirtualMachine) -> PyResult<Option<PyListRef>> {
            let stream = self.connection.read();
            unsafe {
                let chain = sys::SSL_get0_verified_chain(stream.ssl().as_ptr());
                if chain.is_null() {
                    return Ok(None);
                }

                let num_certs = sys::OPENSSL_sk_num(chain as *const _);

                let mut certs = Vec::with_capacity(num_certs as usize);
                // Return Certificate objects
                for i in 0..num_certs {
                    let cert_ptr = sys::OPENSSL_sk_value(chain as *const _, i) as *mut sys::X509;
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
        }

        #[pymethod]
        fn version(&self) -> Option<&'static str> {
            let v = self.connection.read().ssl().version_str();
            if v == "unknown" { None } else { Some(v) }
        }

        #[pymethod]
        fn cipher(&self) -> Option<CipherTuple> {
            self.connection
                .read()
                .ssl()
                .current_cipher()
                .map(cipher_to_tuple)
        }

        #[pymethod]
        fn shared_ciphers(&self, vm: &VirtualMachine) -> Option<PyListRef> {
            #[cfg(ossl110)]
            {
                let stream = self.connection.read();
                unsafe {
                    let server_ciphers = SSL_get_ciphers(stream.ssl().as_ptr());
                    if server_ciphers.is_null() {
                        return None;
                    }

                    let client_ciphers = SSL_get_client_ciphers(stream.ssl().as_ptr());
                    if client_ciphers.is_null() {
                        return None;
                    }

                    let mut result = Vec::new();
                    let num_server = sys::OPENSSL_sk_num(server_ciphers as *const _);
                    let num_client = sys::OPENSSL_sk_num(client_ciphers as *const _);

                    for i in 0..num_server {
                        let server_cipher_ptr = sys::OPENSSL_sk_value(server_ciphers as *const _, i)
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
                            let cipher = ssl::SslCipherRef::from_ptr(server_cipher_ptr as *mut _);
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
            }
            #[cfg(not(ossl110))]
            {
                let _ = vm;
                None
            }
        }

        #[pymethod]
        fn selected_alpn_protocol(&self) -> Option<String> {
            #[cfg(ossl102)]
            {
                let stream = self.connection.read();
                unsafe {
                    let mut out: *const libc::c_uchar = std::ptr::null();
                    let mut outlen: libc::c_uint = 0;

                    sys::SSL_get0_alpn_selected(stream.ssl().as_ptr(), &mut out, &mut outlen);

                    if out.is_null() {
                        None
                    } else {
                        let slice = std::slice::from_raw_parts(out, outlen as usize);
                        Some(String::from_utf8_lossy(slice).into_owned())
                    }
                }
            }
            #[cfg(not(ossl102))]
            {
                None
            }
        }

        #[pymethod]
        fn get_channel_binding(
            &self,
            cb_type: OptionalArg<PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyBytesRef>> {
            const CB_MAXLEN: usize = 512;

            let cb_type_str = cb_type.as_ref().map_or("tls-unique", |s| s.as_str());

            if cb_type_str != "tls-unique" {
                return Err(vm.new_value_error(format!(
                    "Unsupported channel binding type '{}'",
                    cb_type_str
                )));
            }

            let stream = self.connection.read();
            let ssl_ptr = stream.ssl().as_ptr();

            unsafe {
                let session_reused = sys::SSL_session_reused(ssl_ptr) != 0;
                let is_client = matches!(self.socket_type, SslServerOrClient::Client);

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
        }

        #[pymethod]
        fn verify_client_post_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
            #[cfg(ossl111)]
            {
                let stream = self.connection.read();
                let result = unsafe { SSL_verify_client_post_handshake(stream.ssl().as_ptr()) };
                if result == 0 {
                    Err(convert_openssl_error(vm, openssl::error::ErrorStack::get()))
                } else {
                    Ok(())
                }
            }
            #[cfg(not(ossl111))]
            {
                Err(vm.new_not_implemented_error(
                    "Post-handshake auth is not supported by your OpenSSL version.".to_owned(),
                ))
            }
        }

        #[pymethod]
        fn shutdown(&self, vm: &VirtualMachine) -> PyResult<PyRef<PySocket>> {
            let stream = self.connection.read();

            // BIO mode doesn't have an underlying socket
            if stream.is_bio() {
                return Err(vm.new_not_implemented_error(
                    "shutdown() is not supported for BIO-based SSL objects".to_owned(),
                ));
            }

            let ssl_ptr = stream.ssl().as_ptr();

            // Perform SSL shutdown
            let ret = unsafe { sys::SSL_shutdown(ssl_ptr) };

            if ret < 0 {
                // Error occurred
                let err = unsafe { sys::SSL_get_error(ssl_ptr, ret) };

                if err == sys::SSL_ERROR_WANT_READ || err == sys::SSL_ERROR_WANT_WRITE {
                    // Non-blocking would block - this is okay for shutdown
                    // Return the underlying socket
                } else {
                    return Err(vm.new_exception_msg(
                        PySslError::class(&vm.ctx).to_owned(),
                        format!("SSL shutdown failed: error code {}", err),
                    ));
                }
            }

            // Return the underlying socket
            // Get the socket from the stream (SocketStream wraps PyRef<PySocket>)
            let socket = stream
                .get_ref()
                .expect("unwrap() called on bio mode; should only be called in socket mode");
            Ok(socket.0.clone())
        }

        #[cfg(osslconf = "OPENSSL_NO_COMP")]
        #[pymethod]
        fn compression(&self) -> Option<&'static str> {
            None
        }
        #[cfg(not(osslconf = "OPENSSL_NO_COMP"))]
        #[pymethod]
        fn compression(&self) -> Option<&'static str> {
            let stream = self.connection.read();
            let comp_method = unsafe { sys::SSL_get_current_compression(stream.ssl().as_ptr()) };
            if comp_method.is_null() {
                return None;
            }
            let typ = unsafe { sys::COMP_get_type(comp_method) };
            let nid = Nid::from_raw(typ);
            if nid == Nid::UNDEF {
                return None;
            }
            nid.short_name().ok()
        }

        #[pymethod]
        fn do_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
            let mut stream = self.connection.write();
            let ssl_ptr = stream.ssl().as_ptr();

            // BIO mode: no timeout/select logic, just do handshake
            if stream.is_bio() {
                return stream.do_handshake().map_err(|e| {
                    let exc = convert_ssl_error(vm, e);
                    // If it's a cert verification error, set verify info
                    if exc.class().is(PySslCertVerificationError::class(&vm.ctx)) {
                        set_verify_error_info(&exc, ssl_ptr, vm);
                    }
                    exc
                });
            }

            // Socket mode: handle timeout and blocking
            let timeout = stream
                .get_ref()
                .expect("handshake called in bio mode; should only be called in socket mode")
                .timeout_deadline();
            loop {
                let err = match stream.do_handshake() {
                    Ok(()) => return Ok(()),
                    Err(e) => e,
                };
                let (needs, state) = stream
                    .get_ref()
                    .expect("handshake called in bio mode; should only be called in socket mode")
                    .socket_needs(&err, &timeout);
                match state {
                    SelectRet::TimedOut => {
                        return Err(socket::timeout_error_msg(
                            vm,
                            "The handshake operation timed out".to_owned(),
                        ));
                    }
                    SelectRet::Closed => return Err(socket_closed_error(vm)),
                    SelectRet::Nonblocking => {}
                    SelectRet::IsBlocking | SelectRet::Ok => {
                        // For blocking sockets, select() has completed successfully
                        // Continue the handshake loop (matches CPython's SOCKET_IS_BLOCKING behavior)
                        if needs.is_some() {
                            continue;
                        }
                    }
                }
                let exc = convert_ssl_error(vm, err);
                // If it's a cert verification error, set verify info
                if exc.class().is(PySslCertVerificationError::class(&vm.ctx)) {
                    set_verify_error_info(&exc, ssl_ptr, vm);
                }
                return Err(exc);
            }
        }

        #[pymethod]
        fn write(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            let mut stream = self.connection.write();
            let data = data.borrow_buf();
            let data = &*data;

            // BIO mode: no timeout/select logic
            if stream.is_bio() {
                return stream.ssl_write(data).map_err(|e| convert_ssl_error(vm, e));
            }

            // Socket mode: handle timeout and blocking
            let socket_ref = stream
                .get_ref()
                .expect("write called in bio mode; should only be called in socket mode");
            let timeout = socket_ref.timeout_deadline();
            let state = socket_ref.select(SslNeeds::Write, &timeout);
            match state {
                SelectRet::TimedOut => {
                    return Err(socket::timeout_error_msg(
                        vm,
                        "The write operation timed out".to_owned(),
                    ));
                }
                SelectRet::Closed => return Err(socket_closed_error(vm)),
                _ => {}
            }
            loop {
                let err = match stream.ssl_write(data) {
                    Ok(len) => return Ok(len),
                    Err(e) => e,
                };
                let (needs, state) = stream
                    .get_ref()
                    .expect("write called in bio mode; should only be called in socket mode")
                    .socket_needs(&err, &timeout);
                match state {
                    SelectRet::TimedOut => {
                        return Err(socket::timeout_error_msg(
                            vm,
                            "The write operation timed out".to_owned(),
                        ));
                    }
                    SelectRet::Closed => return Err(socket_closed_error(vm)),
                    SelectRet::Nonblocking => {}
                    SelectRet::IsBlocking | SelectRet::Ok => {
                        // For blocking sockets, select() has completed successfully
                        // Continue the write loop (matches CPython's SOCKET_IS_BLOCKING behavior)
                        if needs.is_some() {
                            continue;
                        }
                    }
                }
                return Err(convert_ssl_error(vm, err));
            }
        }

        #[pygetset]
        fn session(&self, _vm: &VirtualMachine) -> PyResult<Option<PySslSession>> {
            let stream = self.connection.read();
            unsafe {
                // Use SSL_get1_session which returns an owned reference (ref count already incremented)
                let session_ptr = SSL_get1_session(stream.ssl().as_ptr());
                if session_ptr.is_null() {
                    Ok(None)
                } else {
                    Ok(Some(PySslSession {
                        session: session_ptr,
                        ctx: self.ctx.read().clone(),
                    }))
                }
            }
        }

        #[pygetset(setter)]
        fn set_session(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            // Check if value is SSLSession type
            let session = value
                .downcast_ref::<PySslSession>()
                .ok_or_else(|| vm.new_type_error("Value is not a SSLSession.".to_owned()))?;

            // Check if session refers to the same SSLContext
            if !std::ptr::eq(
                self.ctx.read().ctx.read().as_ptr(),
                session.ctx.ctx.read().as_ptr(),
            ) {
                return Err(
                    vm.new_value_error("Session refers to a different SSLContext.".to_owned())
                );
            }

            // Check if this is a client socket
            if self.socket_type != SslServerOrClient::Client {
                return Err(
                    vm.new_value_error("Cannot set session for server-side SSLSocket.".to_owned())
                );
            }

            // Check if handshake is not finished
            let stream = self.connection.read();
            unsafe {
                if sys::SSL_is_init_finished(stream.ssl().as_ptr()) != 0 {
                    return Err(
                        vm.new_value_error("Cannot set session after handshake.".to_owned())
                    );
                }

                let ret = sys::SSL_set_session(stream.ssl().as_ptr(), session.session);
                if ret == 0 {
                    return Err(convert_openssl_error(vm, ErrorStack::get()));
                }
            }

            Ok(())
        }

        #[pygetset]
        fn session_reused(&self) -> bool {
            let stream = self.connection.read();
            unsafe { sys::SSL_session_reused(stream.ssl().as_ptr()) != 0 }
        }

        #[pymethod]
        fn read(
            &self,
            n: usize,
            buffer: OptionalArg<ArgMemoryBuffer>,
            vm: &VirtualMachine,
        ) -> PyResult {
            // Special case: reading 0 bytes should return empty bytes immediately
            if n == 0 {
                return if buffer.is_present() {
                    Ok(vm.ctx.new_int(0).into())
                } else {
                    Ok(vm.ctx.new_bytes(vec![]).into())
                };
            }

            let mut stream = self.connection.write();
            let mut inner_buffer = if let OptionalArg::Present(buffer) = &buffer {
                Either::A(buffer.borrow_buf_mut())
            } else {
                Either::B(vec![0u8; n])
            };
            let buf = match &mut inner_buffer {
                Either::A(b) => &mut **b,
                Either::B(b) => b.as_mut_slice(),
            };
            let buf = match buf.get_mut(..n) {
                Some(b) => b,
                None => buf,
            };

            // BIO mode: no timeout/select logic
            let count = if stream.is_bio() {
                match stream.ssl_read(buf) {
                    Ok(count) => count,
                    Err(e) => return Err(convert_ssl_error(vm, e)),
                }
            } else {
                // Socket mode: handle timeout and blocking
                let timeout = stream
                    .get_ref()
                    .expect("read called in bio mode; should only be called in socket mode")
                    .timeout_deadline();
                loop {
                    let err = match stream.ssl_read(buf) {
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
                        .socket_needs(&err, &timeout);
                    match state {
                        SelectRet::TimedOut => {
                            return Err(socket::timeout_error_msg(
                                vm,
                                "The read operation timed out".to_owned(),
                            ));
                        }
                        SelectRet::Closed => return Err(socket_closed_error(vm)),
                        SelectRet::Nonblocking => {}
                        SelectRet::IsBlocking | SelectRet::Ok => {
                            // For blocking sockets, select() has completed successfully
                            // Continue the read loop (matches CPython's SOCKET_IS_BLOCKING behavior)
                            if needs.is_some() {
                                continue;
                            }
                        }
                    }
                    return Err(convert_ssl_error(vm, err));
                }
            };
            let ret = match inner_buffer {
                Either::A(_buf) => vm.ctx.new_int(count).into(),
                Either::B(mut buf) => {
                    buf.truncate(count);
                    buf.shrink_to_fit();
                    vm.ctx.new_bytes(buf).into()
                }
            };
            Ok(ret)
        }
    }

    #[pyattr]
    #[pyclass(module = "ssl", name = "SSLSession")]
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
                let mut self_len: libc::c_uint = 0;
                let mut other_len: libc::c_uint = 0;
                let self_id = sys::SSL_SESSION_get_id(zelf.session, &mut self_len);
                let other_id = sys::SSL_SESSION_get_id(other.session, &mut other_len);

                if self_len != other_len {
                    false
                } else {
                    let self_slice = std::slice::from_raw_parts(self_id, self_len as usize);
                    let other_slice = std::slice::from_raw_parts(other_id, other_len as usize);
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
    #[pyclass(module = "ssl", name = "MemoryBIO")]
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

    // OpenSSL functions not in openssl-sys

    unsafe extern "C" {
        // X509_check_ca returns 1 for CA certificates, 0 otherwise
        fn X509_check_ca(x: *const sys::X509) -> libc::c_int;
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
        fn SSL_verify_client_post_handshake(ssl: *const sys::SSL) -> libc::c_int;
        fn SSL_set_post_handshake_auth(ssl: *mut sys::SSL, val: libc::c_int);
    }

    #[cfg(ossl110)]
    unsafe extern "C" {
        fn SSL_CTX_get_security_level(ctx: *const sys::SSL_CTX) -> libc::c_int;
    }

    unsafe extern "C" {
        fn SSL_set_SSL_CTX(ssl: *mut sys::SSL, ctx: *mut sys::SSL_CTX) -> *mut sys::SSL_CTX;
    }

    // Message callback type
    #[allow(non_camel_case_types)]
    type SSL_CTX_msg_callback = Option<
        unsafe extern "C" fn(
            write_p: libc::c_int,
            version: libc::c_int,
            content_type: libc::c_int,
            buf: *const libc::c_void,
            len: usize,
            ssl: *mut sys::SSL,
            arg: *mut libc::c_void,
        ),
    >;

    unsafe extern "C" {
        fn SSL_CTX_set_msg_callback(ctx: *mut sys::SSL_CTX, cb: SSL_CTX_msg_callback);
    }

    #[cfg(ossl110)]
    unsafe extern "C" {
        fn SSL_SESSION_has_ticket(session: *const sys::SSL_SESSION) -> libc::c_int;
        fn SSL_SESSION_get_ticket_lifetime_hint(session: *const sys::SSL_SESSION) -> libc::c_ulong;
    }

    // X509 object types
    const X509_LU_X509: libc::c_int = 1;
    const X509_LU_CRL: libc::c_int = 2;

    unsafe extern "C" {
        fn X509_OBJECT_get_type(obj: *const sys::X509_OBJECT) -> libc::c_int;
        fn SSL_set_session_id_context(
            ssl: *mut sys::SSL,
            sid_ctx: *const libc::c_uchar,
            sid_ctx_len: libc::c_uint,
        ) -> libc::c_int;
        fn SSL_get1_session(ssl: *const sys::SSL) -> *mut sys::SSL_SESSION;
    }

    // SSL session statistics constants (used with SSL_CTX_ctrl)
    const SSL_CTRL_SESS_NUMBER: libc::c_int = 20;
    const SSL_CTRL_SESS_CONNECT: libc::c_int = 21;
    const SSL_CTRL_SESS_CONNECT_GOOD: libc::c_int = 22;
    const SSL_CTRL_SESS_CONNECT_RENEGOTIATE: libc::c_int = 23;
    const SSL_CTRL_SESS_ACCEPT: libc::c_int = 24;
    const SSL_CTRL_SESS_ACCEPT_GOOD: libc::c_int = 25;
    const SSL_CTRL_SESS_ACCEPT_RENEGOTIATE: libc::c_int = 26;
    const SSL_CTRL_SESS_HIT: libc::c_int = 27;
    const SSL_CTRL_SESS_MISSES: libc::c_int = 29;
    const SSL_CTRL_SESS_TIMEOUTS: libc::c_int = 30;
    const SSL_CTRL_SESS_CACHE_FULL: libc::c_int = 31;

    // SSL session statistics functions (implemented as macros in OpenSSL)
    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_number(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe { sys::SSL_CTX_ctrl(ctx as *mut _, SSL_CTRL_SESS_NUMBER, 0, std::ptr::null_mut()) }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_connect(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_CONNECT,
                0,
                std::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_connect_good(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_CONNECT_GOOD,
                0,
                std::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_connect_renegotiate(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_CONNECT_RENEGOTIATE,
                0,
                std::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_accept(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe { sys::SSL_CTX_ctrl(ctx as *mut _, SSL_CTRL_SESS_ACCEPT, 0, std::ptr::null_mut()) }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_accept_good(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_ACCEPT_GOOD,
                0,
                std::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_accept_renegotiate(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_ACCEPT_RENEGOTIATE,
                0,
                std::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_hits(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe { sys::SSL_CTX_ctrl(ctx as *mut _, SSL_CTRL_SESS_HIT, 0, std::ptr::null_mut()) }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_misses(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe { sys::SSL_CTX_ctrl(ctx as *mut _, SSL_CTRL_SESS_MISSES, 0, std::ptr::null_mut()) }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_timeouts(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_TIMEOUTS,
                0,
                std::ptr::null_mut(),
            )
        }
    }

    #[allow(non_snake_case)]
    unsafe fn SSL_CTX_sess_cache_full(ctx: *const sys::SSL_CTX) -> libc::c_long {
        unsafe {
            sys::SSL_CTX_ctrl(
                ctx as *mut _,
                SSL_CTRL_SESS_CACHE_FULL,
                0,
                std::ptr::null_mut(),
            )
        }
    }

    // DH parameters functions
    unsafe extern "C" {
        fn PEM_read_DHparams(
            fp: *mut libc::FILE,
            x: *mut *mut sys::DH,
            cb: *mut libc::c_void,
            u: *mut libc::c_void,
        ) -> *mut sys::DH;
    }

    // OpenSSL BIO helper functions
    // These are typically macros in OpenSSL, implemented via BIO_ctrl
    const BIO_CTRL_PENDING: libc::c_int = 10;
    const BIO_CTRL_SET_EOF: libc::c_int = 2;

    #[allow(non_snake_case)]
    unsafe fn BIO_ctrl_pending(bio: *mut sys::BIO) -> usize {
        unsafe { sys::BIO_ctrl(bio, BIO_CTRL_PENDING, 0, std::ptr::null_mut()) as usize }
    }

    #[allow(non_snake_case)]
    unsafe fn BIO_set_mem_eof_return(bio: *mut sys::BIO, eof: libc::c_int) -> libc::c_int {
        unsafe {
            sys::BIO_ctrl(
                bio,
                BIO_CTRL_SET_EOF,
                eof as libc::c_long,
                std::ptr::null_mut(),
            ) as libc::c_int
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

        fn py_new(cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
            unsafe {
                let bio = sys::BIO_new(sys::BIO_s_mem());
                if bio.is_null() {
                    return Err(vm.new_memory_error("failed to allocate BIO".to_owned()));
                }

                sys::BIO_set_retry_read(bio);
                BIO_set_mem_eof_return(bio, -1);

                PySslMemoryBio {
                    bio,
                    eof_written: AtomicCell::new(false),
                }
                .into_ref_with_type(vm, cls)
                .map(Into::into)
            }
        }
    }

    #[pyclass(flags(IMMUTABLETYPE), with(Constructor))]
    impl PySslMemoryBio {
        #[pygetset]
        fn pending(&self) -> usize {
            unsafe { BIO_ctrl_pending(self.bio) }
        }

        #[pygetset]
        fn eof(&self) -> bool {
            let pending = unsafe { BIO_ctrl_pending(self.bio) };
            pending == 0 && self.eof_written.load()
        }

        #[pymethod]
        fn read(&self, size: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            unsafe {
                let avail = BIO_ctrl_pending(self.bio).min(i32::MAX as usize) as i32;
                let len = size.unwrap_or(-1);
                let len = if len < 0 || len > avail { avail } else { len };

                // Check if EOF has been written and no data available
                // This matches CPython's behavior where read() returns b'' when EOF is set
                if len == 0 && self.eof_written.load() {
                    return Ok(Vec::new());
                }

                if len == 0 {
                    // No data available and no EOF - would block
                    // Call BIO_read() to get the proper error (SSL_ERROR_WANT_READ)
                    let mut test_buf = [0u8; 1];
                    let nbytes = sys::BIO_read(self.bio, test_buf.as_mut_ptr() as *mut _, 1);
                    if nbytes < 0 {
                        return Err(convert_openssl_error(vm, ErrorStack::get()));
                    }
                    // Shouldn't reach here, but if we do, return what we got
                    return Ok(test_buf[..nbytes as usize].to_vec());
                }

                let mut buf = vec![0u8; len as usize];
                let nbytes = sys::BIO_read(self.bio, buf.as_mut_ptr() as *mut _, len);

                if nbytes < 0 {
                    return Err(convert_openssl_error(vm, ErrorStack::get()));
                }

                buf.truncate(nbytes as usize);
                Ok(buf)
            }
        }

        #[pymethod]
        fn write(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<i32> {
            if self.eof_written.load() {
                return Err(vm.new_exception_msg(
                    PySslError::class(&vm.ctx).to_owned(),
                    "cannot write() after write_eof()".to_owned(),
                ));
            }

            data.with_ref(|buf| unsafe {
                if buf.len() > i32::MAX as usize {
                    return Err(
                        vm.new_overflow_error(format!("string longer than {} bytes", i32::MAX))
                    );
                }

                let nbytes = sys::BIO_write(self.bio, buf.as_ptr() as *const _, buf.len() as i32);
                if nbytes < 0 {
                    return Err(convert_openssl_error(vm, ErrorStack::get()));
                }

                Ok(nbytes)
            })
        }

        #[pymethod]
        fn write_eof(&self) {
            self.eof_written.store(true);
            unsafe {
                BIO_clear_retry_flags(self.bio);
                BIO_set_mem_eof_return(self.bio, 0);
            }
        }
    }

    #[pyclass(flags(IMMUTABLETYPE), with(Comparable))]
    impl PySslSession {
        #[pygetset]
        fn time(&self) -> i64 {
            unsafe {
                #[cfg(ossl330)]
                {
                    sys::SSL_SESSION_get_time(self.session) as i64
                }
                #[cfg(not(ossl330))]
                {
                    sys::SSL_SESSION_get_time(self.session) as i64
                }
            }
        }

        #[pygetset]
        fn timeout(&self) -> i64 {
            unsafe { sys::SSL_SESSION_get_timeout(self.session) as i64 }
        }

        #[pygetset]
        fn ticket_lifetime_hint(&self) -> u64 {
            // SSL_SESSION_get_ticket_lifetime_hint available in OpenSSL 1.1.0+
            #[cfg(ossl110)]
            {
                unsafe { SSL_SESSION_get_ticket_lifetime_hint(self.session) as u64 }
            }
            #[cfg(not(ossl110))]
            {
                // Not available in older OpenSSL versions
                0
            }
        }

        #[pygetset]
        fn id(&self, vm: &VirtualMachine) -> PyBytesRef {
            unsafe {
                let mut len: libc::c_uint = 0;
                let id_ptr = sys::SSL_SESSION_get_id(self.session, &mut len);
                let id_slice = std::slice::from_raw_parts(id_ptr, len as usize);
                vm.ctx.new_bytes(id_slice.to_vec())
            }
        }

        #[pygetset]
        fn has_ticket(&self) -> bool {
            // SSL_SESSION_has_ticket available in OpenSSL 1.1.0+
            #[cfg(ossl110)]
            {
                unsafe { SSL_SESSION_has_ticket(self.session) != 0 }
            }
            #[cfg(not(ossl110))]
            {
                // Not available in older OpenSSL versions
                false
            }
        }
    }

    #[track_caller]
    pub(crate) fn convert_openssl_error(
        vm: &VirtualMachine,
        err: ErrorStack,
    ) -> PyBaseExceptionRef {
        match err.errors().last() {
            Some(e) => {
                // Check if this is a system library error (errno-based)
                let lib = sys::ERR_GET_LIB(e.code());

                if lib == sys::ERR_LIB_SYS {
                    // A system error is being reported; reason is set to errno
                    let reason = sys::ERR_GET_REASON(e.code());

                    // errno 2 = ENOENT = FileNotFoundError
                    let exc_type = if reason == 2 {
                        vm.ctx.exceptions.file_not_found_error.to_owned()
                    } else {
                        vm.ctx.exceptions.os_error.to_owned()
                    };
                    let exc = vm.new_exception(exc_type, vec![vm.ctx.new_int(reason).into()]);
                    // Set errno attribute explicitly
                    let _ = exc
                        .as_object()
                        .set_attr("errno", vm.ctx.new_int(reason), vm);
                    return exc;
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
                    PySslCertVerificationError::class(&vm.ctx).to_owned()
                } else {
                    PySslError::class(&vm.ctx).to_owned()
                };

                // Build message
                let msg = if let Some(lib_str) = lib_name {
                    format!("[{lib_str}] {errstr} ({file}:{line})")
                } else {
                    format!("{errstr} ({file}:{line})")
                };

                // Create exception instance
                let reason = sys::ERR_GET_REASON(e.code());
                let exc = vm.new_exception(
                    cls,
                    vec![vm.ctx.new_int(reason).into(), vm.ctx.new_str(msg).into()],
                );

                // Set attributes on instance, not class
                let exc_obj: PyObjectRef = exc.into();

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
                let cls = PySslError::class(&vm.ctx).to_owned();
                vm.new_exception_empty(cls)
            }
        }
    }

    // Helper function to set verify_code and verify_message on SSLCertVerificationError
    fn set_verify_error_info(
        exc: &PyBaseExceptionRef,
        ssl_ptr: *const sys::SSL,
        vm: &VirtualMachine,
    ) {
        // Get verify result
        let verify_code = unsafe { sys::SSL_get_verify_result(ssl_ptr) };
        let verify_code_obj = vm.ctx.new_int(verify_code);

        // Get verify message
        let verify_message = unsafe {
            let verify_str = sys::X509_verify_cert_error_string(verify_code);
            if verify_str.is_null() {
                vm.ctx.none()
            } else {
                let c_str = std::ffi::CStr::from_ptr(verify_str);
                vm.ctx.new_str(c_str.to_string_lossy()).into()
            }
        };

        let exc_obj = exc.as_object();
        let _ = exc_obj.set_attr("verify_code", verify_code_obj, vm);
        let _ = exc_obj.set_attr("verify_message", verify_message, vm);
    }
    #[track_caller]
    fn convert_ssl_error(
        vm: &VirtualMachine,
        e: impl std::borrow::Borrow<ssl::Error>,
    ) -> PyBaseExceptionRef {
        let e = e.borrow();
        let (cls, msg) = match e.code() {
            ssl::ErrorCode::WANT_READ => (
                PySslWantReadError::class(&vm.ctx).to_owned(),
                "The operation did not complete (read)",
            ),
            ssl::ErrorCode::WANT_WRITE => (
                PySslWantWriteError::class(&vm.ctx).to_owned(),
                "The operation did not complete (write)",
            ),
            ssl::ErrorCode::SYSCALL => match e.io_error() {
                Some(io_err) => return io_err.to_pyexception(vm),
                // When no I/O error and OpenSSL error queue is empty,
                // this is an EOF in violation of protocol -> SSLEOFError
                // Need to set args[0] = SSL_ERROR_EOF for suppress_ragged_eofs check
                None => {
                    return vm.new_exception(
                        PySslEOFError::class(&vm.ctx).to_owned(),
                        vec![
                            vm.ctx.new_int(SSL_ERROR_EOF).into(),
                            vm.ctx
                                .new_str("EOF occurred in violation of protocol")
                                .into(),
                        ],
                    );
                }
            },
            ssl::ErrorCode::SSL => {
                // Check for OpenSSL 3.0 SSL_R_UNEXPECTED_EOF_WHILE_READING
                if let Some(ssl_err) = e.ssl_error() {
                    // In OpenSSL 3.0+, unexpected EOF is reported as SSL_ERROR_SSL
                    // with this specific reason code instead of SSL_ERROR_SYSCALL
                    unsafe {
                        let err_code = sys::ERR_peek_last_error();
                        let reason = sys::ERR_GET_REASON(err_code);
                        let lib = sys::ERR_GET_LIB(err_code);
                        if lib == ERR_LIB_SSL && reason == SSL_R_UNEXPECTED_EOF_WHILE_READING {
                            return vm.new_exception(
                                PySslEOFError::class(&vm.ctx).to_owned(),
                                vec![
                                    vm.ctx.new_int(SSL_ERROR_EOF).into(),
                                    vm.ctx
                                        .new_str("EOF occurred in violation of protocol")
                                        .into(),
                                ],
                            );
                        }
                    }
                    return convert_openssl_error(vm, ssl_err.clone());
                }
                (
                    PySslError::class(&vm.ctx).to_owned(),
                    "A failure in the SSL library occurred",
                )
            }
            _ => (
                PySslError::class(&vm.ctx).to_owned(),
                "A failure in the SSL library occurred",
            ),
        };
        vm.new_exception_msg(cls, msg.to_owned())
    }

    // SSL_FILETYPE_ASN1 part of _add_ca_certs in CPython
    fn x509_stack_from_der(der: &[u8]) -> Result<Vec<X509>, ErrorStack> {
        unsafe {
            openssl::init();
            let bio = bio::MemBioSlice::new(der)?;

            let mut certs = vec![];

            loop {
                let cert = sys::d2i_X509_bio(bio.as_ptr(), std::ptr::null_mut());
                if cert.is_null() {
                    break;
                }
                certs.push(X509::from_ptr(cert));
            }

            if certs.is_empty() {
                // No certificates loaded at all
                return Err(ErrorStack::get());
            }

            // Successfully loaded at least one certificate from DER data.
            // Clear any trailing errors from EOF.
            // CPython clears errors when:
            // - DER: was_bio_eof is set (EOF reached)
            // - PEM: PEM_R_NO_START_LINE error (normal EOF)
            // Both cases mean successful completion with loaded certs.
            eprintln!(
                "[x509_stack_from_der] SUCCESS: Clearing errors and returning {} certs",
                certs.len()
            );
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
                buf.as_mut_ptr() as *mut libc::c_char,
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

    impl Read for SocketStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let mut socket: &PySocket = &self.0;
            socket.read(buf)
        }
    }

    impl Write for SocketStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let mut socket: &PySocket = &self.0;
            socket.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            let mut socket: &PySocket = &self.0;
            socket.flush()
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
        use std::{
            fs::{File, read_dir},
            io::Read,
            path::Path,
        };

        static CERT_DIR: &'static str = "/system/etc/security/cacerts";

        pub(super) fn load_client_ca_list(
            vm: &VirtualMachine,
            b: &mut SslContextBuilder,
        ) -> Result<(), PyBaseExceptionRef> {
            let root = Path::new(CERT_DIR);
            if !root.is_dir() {
                return Err(vm.new_exception_msg(
                    vm.ctx.exceptions.file_not_found_error.to_owned(),
                    CERT_DIR.to_string(),
                ));
            }

            let mut combined_pem = String::new();
            let entries = read_dir(root)
                .map_err(|err| vm.new_os_error(format!("read cert root: {}", err)))?;
            for entry in entries {
                let entry =
                    entry.map_err(|err| vm.new_os_error(format!("iter cert root: {}", err)))?;

                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                File::open(&path)
                    .and_then(|mut file| file.read_to_string(&mut combined_pem))
                    .map_err(|err| {
                        vm.new_os_error(format!("open cert file {}: {}", path.display(), err))
                    })?;

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
}

#[cfg(not(ossl101))]
#[pymodule(sub)]
mod ossl101 {}

#[cfg(not(ossl111))]
#[pymodule(sub)]
mod ossl111 {}

#[cfg(not(windows))]
#[pymodule(sub)]
mod windows {}

#[allow(non_upper_case_globals)]
#[cfg(ossl101)]
#[pymodule(sub)]
mod ossl101 {
    #[pyattr]
    use openssl_sys::{
        SSL_OP_NO_COMPRESSION as OP_NO_COMPRESSION, SSL_OP_NO_TLSv1_1 as OP_NO_TLSv1_1,
        SSL_OP_NO_TLSv1_2 as OP_NO_TLSv1_2,
    };
}

#[allow(non_upper_case_globals)]
#[cfg(ossl111)]
#[pymodule(sub)]
mod ossl111 {
    #[pyattr]
    use openssl_sys::SSL_OP_NO_TLSv1_3 as OP_NO_TLSv1_3;
}

#[cfg(windows)]
#[pymodule(sub)]
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
        use schannel::{RawPointer, cert_context::ValidUses, cert_store::CertStore};
        use windows_sys::Win32::Security::Cryptography;

        // TODO: check every store for it, not just 2 of them:
        // https://github.com/python/cpython/blob/3.8/Modules/_ssl.c#L5603-L5610
        let open_fns = [CertStore::open_current_user, CertStore::open_local_machine];
        let stores = open_fns
            .iter()
            .filter_map(|open| open(store_name.as_str()).ok())
            .collect::<Vec<_>>();
        let certs = stores.iter().flat_map(|s| s.certs()).map(|c| {
            let cert = vm.ctx.new_bytes(c.to_der().to_owned());
            let enc_type = unsafe {
                let ptr = c.as_ptr() as *const Cryptography::CERT_CONTEXT;
                (*ptr).dwCertEncodingType
            };
            let enc_type = match enc_type {
                Cryptography::X509_ASN_ENCODING => vm.new_pyobj(ascii!("x509_asn")),
                Cryptography::PKCS_7_ASN_ENCODING => vm.new_pyobj(ascii!("pkcs_7_asn")),
                other => vm.new_pyobj(other),
            };
            let usage: PyObjectRef = match c.valid_uses().map_err(|e| e.to_pyexception(vm))? {
                ValidUses::All => vm.ctx.new_bool(true).into(),
                ValidUses::Oids(oids) => PyFrozenSet::from_iter(
                    vm,
                    oids.into_iter().map(|oid| vm.ctx.new_str(oid).into()),
                )?
                .into_ref(&vm.ctx)
                .into(),
            };
            Ok(vm.new_tuple((cert, enc_type, usage)).into())
        });
        let certs: Vec<PyObjectRef> = certs.collect::<PyResult<Vec<_>>>()?;
        Ok(certs)
    }
}

mod bio {
    //! based off rust-openssl's private `bio` module

    use libc::c_int;
    use openssl::error::ErrorStack;
    use openssl_sys as sys;
    use std::marker::PhantomData;

    pub struct MemBioSlice<'a>(*mut sys::BIO, PhantomData<&'a [u8]>);

    impl Drop for MemBioSlice<'_> {
        fn drop(&mut self) {
            unsafe {
                sys::BIO_free_all(self.0);
            }
        }
    }

    impl<'a> MemBioSlice<'a> {
        pub fn new(buf: &'a [u8]) -> Result<MemBioSlice<'a>, ErrorStack> {
            openssl::init();

            assert!(buf.len() <= c_int::MAX as usize);
            let bio = unsafe { sys::BIO_new_mem_buf(buf.as_ptr() as *const _, buf.len() as c_int) };
            if bio.is_null() {
                return Err(ErrorStack::get());
            }

            Ok(MemBioSlice(bio, PhantomData))
        }

        pub fn as_ptr(&self) -> *mut sys::BIO {
            self.0
        }
    }
}
