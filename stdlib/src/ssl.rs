// spell-checker:disable

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
#[pymodule(with(ossl101, ossl111, windows))]
mod _ssl {
    use super::{bio, probe};
    use crate::{
        common::{
            ascii,
            lock::{
                PyMappedRwLockReadGuard, PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard,
            },
        },
        socket::{self, PySocket},
        vm::{
            Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
            builtins::{
                PyBaseExceptionRef, PyBytesRef, PyListRef, PyStrRef, PyType, PyTypeRef, PyWeak,
            },
            class_or_notimplemented,
            convert::{ToPyException, ToPyObject},
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
        x509::{self, X509, X509Ref},
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

    // Constants
    #[pyattr]
    use sys::{
        // TODO: so many more of these
        SSL_AD_DECODE_ERROR as ALERT_DESCRIPTION_DECODE_ERROR,
        SSL_AD_ILLEGAL_PARAMETER as ALERT_DESCRIPTION_ILLEGAL_PARAMETER,
        SSL_AD_UNRECOGNIZED_NAME as ALERT_DESCRIPTION_UNRECOGNIZED_NAME,
        // SSL_ERROR_INVALID_ERROR_CODE,
        SSL_ERROR_SSL,
        // SSL_ERROR_WANT_X509_LOOKUP,
        SSL_ERROR_SYSCALL,
        SSL_ERROR_WANT_CONNECT,
        SSL_ERROR_WANT_READ,
        SSL_ERROR_WANT_WRITE,
        // X509_V_FLAG_CRL_CHECK as VERIFY_CRL_CHECK_LEAF,
        // sys::X509_V_FLAG_CRL_CHECK|sys::X509_V_FLAG_CRL_CHECK_ALL as VERIFY_CRL_CHECK_CHAIN
        // X509_V_FLAG_X509_STRICT as VERIFY_X509_STRICT,
        SSL_ERROR_ZERO_RETURN,
        SSL_OP_CIPHER_SERVER_PREFERENCE as OP_CIPHER_SERVER_PREFERENCE,
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
    const HAS_SSLv2: bool = true;
    #[pyattr]
    const HAS_SSLv3: bool = true;
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

    /// An error occurred in the SSL implementation.
    #[pyattr(name = "SSLError", once)]
    fn ssl_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "ssl",
            "SSLError",
            Some(vec![vm.ctx.exceptions.os_error.to_owned()]),
        )
    }

    /// A certificate could not be verified.
    #[pyattr(name = "SSLCertVerificationError", once)]
    fn ssl_cert_verification_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "ssl",
            "SSLCertVerificationError",
            Some(vec![
                ssl_error(vm),
                vm.ctx.exceptions.value_error.to_owned(),
            ]),
        )
    }

    /// SSL/TLS session closed cleanly.
    #[pyattr(name = "SSLZeroReturnError", once)]
    fn ssl_zero_return_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("ssl", "SSLZeroReturnError", Some(vec![ssl_error(vm)]))
    }

    /// Non-blocking SSL socket needs to read more data before the requested operation can be completed.
    #[pyattr(name = "SSLWantReadError", once)]
    fn ssl_want_read_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("ssl", "SSLWantReadError", Some(vec![ssl_error(vm)]))
    }

    /// Non-blocking SSL socket needs to write more data before the requested operation can be completed.
    #[pyattr(name = "SSLWantWriteError", once)]
    fn ssl_want_write_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("ssl", "SSLWantWriteError", Some(vec![ssl_error(vm)]))
    }

    /// System error when attempting SSL operation.
    #[pyattr(name = "SSLSyscallError", once)]
    fn ssl_syscall_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("ssl", "SSLSyscallError", Some(vec![ssl_error(vm)]))
    }

    /// SSL/TLS connection terminated abruptly.
    #[pyattr(name = "SSLEOFError", once)]
    fn ssl_eof_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("ssl", "SSLEOFError", Some(vec![ssl_error(vm)]))
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
    fn obj2txt(obj: &Asn1ObjectRef, no_name: bool) -> Option<String> {
        let no_name = i32::from(no_name);
        let ptr = obj.as_ptr();
        let b = unsafe {
            let buflen = sys::OBJ_obj2txt(std::ptr::null_mut(), 0, ptr, no_name);
            assert!(buflen >= 0);
            if buflen == 0 {
                return None;
            }
            let buflen = buflen as usize;
            let mut buf = Vec::<u8>::with_capacity(buflen + 1);
            let ret = sys::OBJ_obj2txt(
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.capacity() as _,
                ptr,
                no_name,
            );
            assert!(ret >= 0);
            // SAFETY: OBJ_obj2txt initialized the buffer successfully
            buf.set_len(buflen);
            buf
        };
        let s = String::from_utf8(b)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        Some(s)
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
        Ok((nid.as_raw(), short_name, long_name, obj2txt(obj, true)))
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
            builder.set_options(options);

            let mode = ssl::SslMode::ACCEPT_MOVING_WRITE_BUFFER | ssl::SslMode::AUTO_RETRY;
            builder.set_mode(mode);

            #[cfg(ossl111)]
            unsafe {
                sys::SSL_CTX_set_post_handshake_auth(builder.as_ptr(), 0);
            }

            builder
                .set_session_id_context(b"Python")
                .map_err(|e| convert_openssl_error(vm, e))?;

            PySslContext {
                ctx: PyRwLock::new(builder),
                check_hostname: AtomicCell::new(check_hostname),
                protocol: proto,
                post_handshake_auth: PyMutex::new(false),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(flags(BASETYPE), with(Constructor))]
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

        #[pymethod]
        fn set_ciphers(&self, cipherlist: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
            let ciphers = cipherlist.as_str();
            if ciphers.contains('\0') {
                return Err(exceptions::cstring_error(vm));
            }
            self.builder().set_cipher_list(ciphers).map_err(|_| {
                vm.new_exception_msg(ssl_error(vm), "No cipher can be selected.".to_owned())
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
                    result.push(dict.into());
                }

                Ok(vm.ctx.new_list(result))
            }
        }

        #[pymethod]
        fn set_ecdh_curve(&self, name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
            use openssl::ec::{EcGroup, EcKey};

            let curve_name = name.as_str();
            if curve_name.contains('\0') {
                return Err(exceptions::cstring_error(vm));
            }

            // Find the NID for the curve name using OBJ_sn2nid
            let name_cstr = name.to_cstring(vm)?;
            let nid_raw = unsafe { sys::OBJ_sn2nid(name_cstr.as_ptr()) };
            if nid_raw == 0 {
                return Err(vm.new_value_error(format!("unknown curve name: {}", curve_name)));
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
                        ssl_error(vm),
                        "Failed to clear verify flags".to_owned(),
                    ));
                }
                if set != 0 && sys::X509_VERIFY_PARAM_set_flags(param, set) == 0 {
                    return Err(vm.new_exception_msg(
                        ssl_error(vm),
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
            if let Some(cafile) = &args.cafile {
                cafile.ensure_no_nul(vm)?
            }
            if let Some(capath) = &args.capath {
                capath.ensure_no_nul(vm)?
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
                ctx.load_verify_locations(
                    args.cafile.as_ref().map(|s| s.as_str().as_ref()),
                    args.capath.as_ref().map(|s| s.as_str().as_ref()),
                )
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

        #[pymethod]
        fn _wrap_socket(
            zelf: PyRef<Self>,
            args: WrapSocketArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PySslSocket> {
            // validate socket type and context protocol
            if !args.server_side && zelf.protocol == SslVersion::TlsServer {
                return Err(vm.new_exception_msg(
                    ssl_error(vm),
                    "Cannot create a client socket with a PROTOCOL_TLS_SERVER context".to_owned(),
                ));
            }
            if args.server_side && zelf.protocol == SslVersion::TlsClient {
                return Err(vm.new_exception_msg(
                    ssl_error(vm),
                    "Cannot create a server socket with a PROTOCOL_TLS_CLIENT context".to_owned(),
                ));
            }

            let mut ssl = ssl::Ssl::new(&zelf.ctx()).map_err(|e| convert_openssl_error(vm, e))?;

            let socket_type = if args.server_side {
                ssl.set_accept_state();
                SslServerOrClient::Server
            } else {
                ssl.set_connect_state();
                SslServerOrClient::Client
            };

            if let Some(hostname) = &args.server_hostname {
                let hostname = hostname.as_str();
                if hostname.is_empty() || hostname.starts_with('.') {
                    return Err(vm.new_value_error(
                        "server_hostname cannot be an empty string or start with a leading dot.",
                    ));
                }
                let ip = hostname.parse::<std::net::IpAddr>();
                if ip.is_err() {
                    ssl.set_hostname(hostname)
                        .map_err(|e| convert_openssl_error(vm, e))?;
                }
                if zelf.check_hostname.load() {
                    if let Ok(ip) = ip {
                        ssl.param_mut()
                            .set_ip(ip)
                            .map_err(|e| convert_openssl_error(vm, e))?;
                    } else {
                        ssl.param_mut()
                            .set_host(hostname)
                            .map_err(|e| convert_openssl_error(vm, e))?;
                    }
                }
            }

            let stream = ssl::SslStream::new(ssl, SocketStream(args.sock.clone()))
                .map_err(|e| convert_openssl_error(vm, e))?;

            let py_ssl_socket = PySslSocket {
                ctx: zelf,
                stream: PyRwLock::new(stream),
                socket_type,
                server_hostname: args.server_hostname,
                owner: PyRwLock::new(args.owner.map(|o| o.downgrade(None, vm)).transpose()?),
            };

            // Set session if provided
            if let Some(session) = args.session
                && !vm.is_none(&session)
            {
                py_ssl_socket.set_session(session, vm)?;
            }

            Ok(py_ssl_socket)
        }

        #[pymethod]
        fn _wrap_bio(_zelf: PyRef<Self>, _args: WrapBioArgs, vm: &VirtualMachine) -> PyResult {
            // TODO: Implement BIO-based SSL wrapping
            // This requires refactoring PySslSocket to support both socket and BIO modes
            Err(vm.new_not_implemented_error(
                "_wrap_bio is not yet implemented in RustPython".to_owned(),
            ))
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
        cafile: Option<PyStrRef>,
        #[pyarg(any, default)]
        capath: Option<PyStrRef>,
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
            ssl_error(vm),
            "Underlying socket has been closed.".to_owned(),
        )
    }

    #[pyattr]
    #[pyclass(module = "ssl", name = "_SSLSocket", traverse)]
    #[derive(PyPayload)]
    struct PySslSocket {
        ctx: PyRef<PySslContext>,
        #[pytraverse(skip)]
        stream: PyRwLock<ssl::SslStream<SocketStream>>,
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

    #[pyclass]
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
            self.ctx.clone()
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
            let stream = self.stream.read();
            if !stream.ssl().is_init_finished() {
                return Err(vm.new_value_error("handshake not done yet"));
            }
            stream
                .ssl()
                .peer_certificate()
                .map(|cert| cert_to_py(vm, &cert, binary))
                .transpose()
        }

        #[pymethod]
        fn get_unverified_chain(&self, vm: &VirtualMachine) -> Option<PyObjectRef> {
            let stream = self.stream.read();
            let chain = stream.ssl().peer_cert_chain()?;

            let certs: Vec<PyObjectRef> = chain
                .iter()
                .filter_map(|cert| cert.to_der().ok().map(|der| vm.ctx.new_bytes(der).into()))
                .collect();

            Some(vm.ctx.new_list(certs).into())
        }

        #[pymethod]
        fn get_verified_chain(&self, vm: &VirtualMachine) -> Option<PyListRef> {
            let stream = self.stream.read();
            unsafe {
                let chain = sys::SSL_get0_verified_chain(stream.ssl().as_ptr());
                if chain.is_null() {
                    return None;
                }

                let num_certs = sys::OPENSSL_sk_num(chain as *const _);
                let mut certs = Vec::new();

                for i in 0..num_certs {
                    let cert_ptr = sys::OPENSSL_sk_value(chain as *const _, i) as *mut sys::X509;
                    if cert_ptr.is_null() {
                        continue;
                    }
                    let cert = X509Ref::from_ptr(cert_ptr);
                    if let Ok(der) = cert.to_der() {
                        certs.push(vm.ctx.new_bytes(der).into());
                    }
                }

                if certs.is_empty() {
                    None
                } else {
                    Some(vm.ctx.new_list(certs))
                }
            }
        }

        #[pymethod]
        fn version(&self) -> Option<&'static str> {
            let v = self.stream.read().ssl().version_str();
            if v == "unknown" { None } else { Some(v) }
        }

        #[pymethod]
        fn cipher(&self) -> Option<CipherTuple> {
            self.stream
                .read()
                .ssl()
                .current_cipher()
                .map(cipher_to_tuple)
        }

        #[pymethod]
        fn shared_ciphers(&self, vm: &VirtualMachine) -> Option<PyListRef> {
            #[cfg(ossl110)]
            {
                let stream = self.stream.read();
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
                let stream = self.stream.read();
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

            let stream = self.stream.read();
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
                let stream = self.stream.read();
                let result = unsafe { SSL_verify_client_post_handshake(stream.ssl().as_ptr()) };
                if result == 0 {
                    Err(vm.new_exception_msg(
                        ssl_error(vm),
                        "Post-handshake authentication failed".to_owned(),
                    ))
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
            let stream = self.stream.read();
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
                        ssl_error(vm),
                        format!("SSL shutdown failed: error code {}", err),
                    ));
                }
            }

            // Return the underlying socket
            // Get the socket from the stream (SocketStream wraps PyRef<PySocket>)
            let socket = stream.get_ref();
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
            let stream = self.stream.read();
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
            let mut stream = self.stream.write();
            let timeout = stream.get_ref().timeout_deadline();
            loop {
                let err = match stream.do_handshake() {
                    Ok(()) => return Ok(()),
                    Err(e) => e,
                };
                let (needs, state) = stream.get_ref().socket_needs(&err, &timeout);
                match state {
                    SelectRet::TimedOut => {
                        return Err(socket::timeout_error_msg(
                            vm,
                            "The handshake operation timed out".to_owned(),
                        ));
                    }
                    SelectRet::Closed => return Err(socket_closed_error(vm)),
                    SelectRet::Nonblocking => {}
                    _ => {
                        if needs.is_some() {
                            continue;
                        }
                    }
                }
                return Err(convert_ssl_error(vm, err));
            }
        }

        #[pymethod]
        fn write(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            let mut stream = self.stream.write();
            let data = data.borrow_buf();
            let data = &*data;
            let timeout = stream.get_ref().timeout_deadline();
            let state = stream.get_ref().select(SslNeeds::Write, &timeout);
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
                let (needs, state) = stream.get_ref().socket_needs(&err, &timeout);
                match state {
                    SelectRet::TimedOut => {
                        return Err(socket::timeout_error_msg(
                            vm,
                            "The write operation timed out".to_owned(),
                        ));
                    }
                    SelectRet::Closed => return Err(socket_closed_error(vm)),
                    SelectRet::Nonblocking => {}
                    _ => {
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
            let stream = self.stream.read();
            unsafe {
                let session_ptr = sys::SSL_get_session(stream.ssl().as_ptr());
                if session_ptr.is_null() {
                    Ok(None)
                } else {
                    // Increment reference count since SSL_get_session returns a borrowed reference
                    #[cfg(ossl110)]
                    let _session = sys::SSL_SESSION_up_ref(session_ptr);

                    Ok(Some(PySslSession {
                        session: session_ptr,
                        ctx: self.ctx.clone(),
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
                self.ctx.ctx.read().as_ptr(),
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
            let stream = self.stream.read();
            unsafe {
                if sys::SSL_is_init_finished(stream.ssl().as_ptr()) != 0 {
                    return Err(
                        vm.new_value_error("Cannot set session after handshake.".to_owned())
                    );
                }

                if sys::SSL_set_session(stream.ssl().as_ptr(), session.session) == 0 {
                    return Err(convert_openssl_error(vm, ErrorStack::get()));
                }
            }

            Ok(())
        }

        #[pygetset]
        fn session_reused(&self) -> bool {
            let stream = self.stream.read();
            unsafe { sys::SSL_session_reused(stream.ssl().as_ptr()) != 0 }
        }

        #[pymethod]
        fn read(
            &self,
            n: usize,
            buffer: OptionalArg<ArgMemoryBuffer>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let mut stream = self.stream.write();
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
            let timeout = stream.get_ref().timeout_deadline();
            let count = loop {
                let err = match stream.ssl_read(buf) {
                    Ok(count) => break count,
                    Err(e) => e,
                };
                if err.code() == ssl::ErrorCode::ZERO_RETURN
                    && stream.get_shutdown() == ssl::ShutdownState::RECEIVED
                {
                    break 0;
                }
                let (needs, state) = stream.get_ref().socket_needs(&err, &timeout);
                match state {
                    SelectRet::TimedOut => {
                        return Err(socket::timeout_error_msg(
                            vm,
                            "The read operation timed out".to_owned(),
                        ));
                    }
                    SelectRet::Nonblocking => {}
                    _ => {
                        if needs.is_some() {
                            continue;
                        }
                    }
                }
                return Err(convert_ssl_error(vm, err));
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

    #[pyclass(with(Constructor))]
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

                if len == 0 {
                    return Ok(Vec::new());
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
                    ssl_error(vm),
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

    #[pyclass(with(Comparable))]
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
            // SSL_SESSION_get_ticket_lifetime_hint may not be available in older OpenSSL
            // Return 0 as default if not available
            #[cfg(ossl110)]
            {
                // For now, return 0 as this function may not be in openssl-sys
                let _ = self.session;
                0
            }
            #[cfg(not(ossl110))]
            {
                let _ = self.session;
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
            // SSL_SESSION_has_ticket may not be available in older OpenSSL
            // Return false as default
            #[cfg(ossl110)]
            {
                // For now, return false as this function may not be in openssl-sys
                let _ = self.session;
                false
            }
            #[cfg(not(ossl110))]
            {
                let _ = self.session;
                false
            }
        }
    }

    #[track_caller]
    fn convert_openssl_error(vm: &VirtualMachine, err: ErrorStack) -> PyBaseExceptionRef {
        let cls = ssl_error(vm);
        match err.errors().last() {
            Some(e) => {
                let caller = std::panic::Location::caller();
                let (file, line) = (caller.file(), caller.line());
                let file = file
                    .rsplit_once(&['/', '\\'][..])
                    .map_or(file, |(_, basename)| basename);
                // TODO: finish map
                let default_errstr = e.reason().unwrap_or("unknown error");
                let errstr = match default_errstr {
                    "certificate verify failed" => "CERTIFICATE_VERIFY_FAILED",
                    _ => default_errstr,
                };

                // Build message
                let lib_obj = e.library();
                let msg = if let Some(lib) = lib_obj {
                    format!("[{lib}] {errstr} ({file}:{line})")
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
                let library_value: PyObjectRef = if let Some(lib) = lib_obj {
                    vm.ctx.new_str(lib).into()
                } else {
                    vm.ctx.none()
                };
                let _ = exc_obj.set_attr("library", library_value, vm);

                // Convert back to PyBaseExceptionRef
                exc_obj.downcast().unwrap()
            }
            None => vm.new_exception_empty(cls),
        }
    }
    #[track_caller]
    fn convert_ssl_error(
        vm: &VirtualMachine,
        e: impl std::borrow::Borrow<ssl::Error>,
    ) -> PyBaseExceptionRef {
        let e = e.borrow();
        let (cls, msg) = match e.code() {
            ssl::ErrorCode::WANT_READ => (
                vm.class("_ssl", "SSLWantReadError"),
                "The operation did not complete (read)",
            ),
            ssl::ErrorCode::WANT_WRITE => (
                vm.class("_ssl", "SSLWantWriteError"),
                "The operation did not complete (write)",
            ),
            ssl::ErrorCode::SYSCALL => match e.io_error() {
                Some(io_err) => return io_err.to_pyexception(vm),
                None => (
                    vm.class("_ssl", "SSLSyscallError"),
                    "EOF occurred in violation of protocol",
                ),
            },
            ssl::ErrorCode::SSL => match e.ssl_error() {
                Some(e) => return convert_openssl_error(vm, e.clone()),
                None => (ssl_error(vm), "A failure in the SSL library occurred"),
            },
            _ => (ssl_error(vm), "A failure in the SSL library occurred"),
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

            let err = sys::ERR_peek_last_error();

            if certs.is_empty() {
                // let msg = if filetype == sys::SSL_FILETYPE_PEM {
                //     "no start line: cadata does not contain a certificate"
                // } else {
                //     "not enough data: cadata does not contain a certificate"
                // };
                return Err(ErrorStack::get());
            }
            if err != 0 {
                return Err(ErrorStack::get());
            }

            Ok(certs)
        }
    }

    type CipherTuple = (&'static str, &'static str, i32);

    fn cipher_to_tuple(cipher: &ssl::SslCipherRef) -> CipherTuple {
        (cipher.name(), cipher.version(), cipher.bits().secret)
    }

    fn cert_to_py(vm: &VirtualMachine, cert: &X509Ref, binary: bool) -> PyResult {
        let r = if binary {
            let b = cert.to_der().map_err(|e| convert_openssl_error(vm, e))?;
            vm.ctx.new_bytes(b).into()
        } else {
            let dict = vm.ctx.new_dict();

            let name_to_py = |name: &x509::X509NameRef| -> PyResult {
                let list = name
                    .entries()
                    .map(|entry| {
                        let txt = obj2txt(entry.object(), false).to_pyobject(vm);
                        let data = vm.ctx.new_str(entry.data().as_utf8()?.to_owned());
                        Ok(vm.new_tuple(((txt, data),)).into())
                    })
                    .collect::<Result<_, _>>()
                    .map_err(|e| convert_openssl_error(vm, e))?;
                Ok(vm.ctx.new_tuple(list).into())
            };

            dict.set_item("subject", name_to_py(cert.subject_name())?, vm)?;
            dict.set_item("issuer", name_to_py(cert.issuer_name())?, vm)?;
            dict.set_item("version", vm.new_pyobj(cert.version()), vm)?;

            let serial_num = cert
                .serial_number()
                .to_bn()
                .and_then(|bn| bn.to_hex_str())
                .map_err(|e| convert_openssl_error(vm, e))?;
            dict.set_item(
                "serialNumber",
                vm.ctx.new_str(serial_num.to_owned()).into(),
                vm,
            )?;

            dict.set_item(
                "notBefore",
                vm.ctx.new_str(cert.not_before().to_string()).into(),
                vm,
            )?;
            dict.set_item(
                "notAfter",
                vm.ctx.new_str(cert.not_after().to_string()).into(),
                vm,
            )?;

            #[allow(clippy::manual_map)]
            if let Some(names) = cert.subject_alt_names() {
                let san = names
                    .iter()
                    .filter_map(|gen_name| {
                        if let Some(email) = gen_name.email() {
                            Some(vm.new_tuple((ascii!("email"), email)).into())
                        } else if let Some(dnsname) = gen_name.dnsname() {
                            Some(vm.new_tuple((ascii!("DNS"), dnsname)).into())
                        } else if let Some(ip) = gen_name.ipaddress() {
                            Some(
                                vm.new_tuple((
                                    ascii!("IP Address"),
                                    String::from_utf8_lossy(ip).into_owned(),
                                ))
                                .into(),
                            )
                        } else {
                            // TODO: convert every type of general name:
                            // https://github.com/python/cpython/blob/3.6/Modules/_ssl.c#L1092-L1231
                            None
                        }
                    })
                    .collect();
                dict.set_item("subjectAltName", vm.ctx.new_tuple(san).into(), vm)?;
            };

            dict.into()
        };
        Ok(r)
    }

    #[pyfunction]
    fn _test_decode_cert(path: FsPath, vm: &VirtualMachine) -> PyResult {
        let path = path.to_path_buf(vm)?;
        let pem = std::fs::read(path).map_err(|e| e.to_pyexception(vm))?;
        let x509 = X509::from_pem(&pem).map_err(|e| convert_openssl_error(vm, e))?;
        cert_to_py(vm, &x509, false)
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
