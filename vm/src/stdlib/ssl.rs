use super::os::PyPathLike;
use super::socket::PySocketRef;
use crate::builtins::{pytype, weakref::PyWeak, PyStrRef, PyTypeRef};
use crate::byteslike::{PyBytesLike, PyRwBytesLike};
use crate::common::lock::{PyRwLock, PyRwLockWriteGuard};
use crate::exceptions::{create_exception_type, IntoPyException, PyBaseExceptionRef};
use crate::function::OptionalArg;
use crate::pyobject::{
    BorrowValue, Either, IntoPyObject, ItemProtocol, PyCallable, PyClassImpl, PyObjectRef, PyRef,
    PyResult, PyValue, StaticType,
};
use crate::VirtualMachine;

use crossbeam_utils::atomic::AtomicCell;
use foreign_types_shared::{ForeignType, ForeignTypeRef};
use openssl::{
    asn1::{Asn1Object, Asn1ObjectRef},
    error::ErrorStack,
    nid::Nid,
    ssl::{self, SslContextBuilder, SslOptions, SslVerifyMode},
    x509::{self, X509Object, X509Ref, X509},
};
use std::convert::TryFrom;
use std::ffi::{CStr, CString};
use std::fmt;
use std::time::Instant;

mod sys {
    #![allow(non_camel_case_types, unused)]
    use libc::{c_char, c_double, c_int, c_long, c_void};
    pub use openssl_sys::*;
    extern "C" {
        pub fn OBJ_txt2obj(s: *const c_char, no_name: c_int) -> *mut ASN1_OBJECT;
        pub fn OBJ_nid2obj(n: c_int) -> *mut ASN1_OBJECT;
        pub fn X509_get_default_cert_file_env() -> *const c_char;
        pub fn X509_get_default_cert_file() -> *const c_char;
        pub fn X509_get_default_cert_dir_env() -> *const c_char;
        pub fn X509_get_default_cert_dir() -> *const c_char;
        pub fn SSL_CTX_set_post_handshake_auth(ctx: *mut SSL_CTX, val: c_int);
        pub fn RAND_add(buf: *const c_void, num: c_int, randomness: c_double);
        pub fn RAND_pseudo_bytes(buf: *const u8, num: c_int) -> c_int;
        pub fn X509_get_version(x: *const X509) -> c_long;
        pub fn SSLv3_method() -> *const SSL_METHOD;
        pub fn TLSv1_method() -> *const SSL_METHOD;
        pub fn COMP_get_type(meth: *const COMP_METHOD) -> i32;
    }
}

#[derive(Copy, Clone, num_enum::IntoPrimitive, num_enum::TryFromPrimitive, PartialEq)]
#[repr(i32)]
enum SslVersion {
    Ssl2,
    Ssl3 = 1,
    Tls,
    Tls1,
    // TODO: Tls1_1, Tls1_2 ?
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

// taken from CPython, should probably be kept up to date with their version if it ever changes
const DEFAULT_CIPHER_STRING: &str =
    "DEFAULT:!aNULL:!eNULL:!MD5:!3DES:!DES:!RC4:!IDEA:!SEED:!aDSS:!SRP:!PSK";

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
        Some(Asn1Object::from_ptr(ptr))
    }
}
fn txt2obj(s: &CStr, no_name: bool) -> Option<Asn1Object> {
    unsafe { ptr2obj(sys::OBJ_txt2obj(s.as_ptr(), if no_name { 1 } else { 0 })) }
}
fn nid2obj(nid: Nid) -> Option<Asn1Object> {
    unsafe { ptr2obj(sys::OBJ_nid2obj(nid.as_raw())) }
}
fn obj2txt(obj: &Asn1ObjectRef, no_name: bool) -> Option<String> {
    unsafe {
        let no_name = if no_name { 1 } else { 0 };
        let ptr = obj.as_ptr();
        let buflen = sys::OBJ_obj2txt(std::ptr::null_mut(), 0, ptr, no_name);
        assert!(buflen >= 0);
        if buflen == 0 {
            return None;
        }
        let mut buf = vec![0u8; buflen as usize];
        let ret = sys::OBJ_obj2txt(buf.as_mut_ptr() as *mut libc::c_char, buflen, ptr, no_name);
        assert!(ret >= 0);
        let s = String::from_utf8(buf)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        Some(s)
    }
}

type PyNid = (libc::c_int, String, String, Option<String>);
fn obj2py(obj: &Asn1ObjectRef) -> PyNid {
    let nid = obj.nid();
    (
        nid.as_raw(),
        nid.short_name().unwrap().to_owned(),
        nid.long_name().unwrap().to_owned(),
        obj2txt(obj, true),
    )
}

#[cfg(windows)]
fn _ssl_enum_certificates(store_name: PyStrRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    use crate::builtins::set::PyFrozenSet;
    use schannel::{cert_context::ValidUses, cert_store::CertStore, RawPointer};
    use winapi::um::wincrypt;
    // TODO: check every store for it, not just 2 of them:
    // https://github.com/python/cpython/blob/3.8/Modules/_ssl.c#L5603-L5610
    let open_fns = [CertStore::open_current_user, CertStore::open_local_machine];
    let stores = open_fns
        .iter()
        .filter_map(|open| open(store_name.borrow_value()).ok())
        .collect::<Vec<_>>();
    let certs = stores.iter().map(|s| s.certs()).flatten().map(|c| {
        let cert = vm.ctx.new_bytes(c.to_der().to_owned());
        let enc_type = unsafe {
            let ptr = c.as_ptr() as wincrypt::PCCERT_CONTEXT;
            (*ptr).dwCertEncodingType
        };
        let enc_type = match enc_type {
            wincrypt::X509_ASN_ENCODING => vm.ctx.new_str("x509_asn"),
            wincrypt::PKCS_7_ASN_ENCODING => vm.ctx.new_str("pkcs_7_asn"),
            other => vm.ctx.new_int(other),
        };
        let usage = match c.valid_uses()? {
            ValidUses::All => vm.ctx.new_bool(true),
            ValidUses::Oids(oids) => {
                PyFrozenSet::from_iter(vm, oids.into_iter().map(|oid| vm.ctx.new_str(oid)))
                    .unwrap()
                    .into_ref(vm)
                    .into_object()
            }
        };
        Ok(vm.ctx.new_tuple(vec![cert, enc_type, usage]))
    });
    let certs = certs
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e: std::io::Error| e.into_pyexception(vm))?;
    Ok(vm.ctx.new_list(certs))
}

#[derive(FromArgs)]
struct Txt2ObjArgs {
    #[pyarg(any)]
    txt: CString,
    #[pyarg(any, default = "false")]
    name: bool,
}
fn _ssl_txt2obj(args: Txt2ObjArgs, vm: &VirtualMachine) -> PyResult<PyNid> {
    txt2obj(&args.txt, !args.name)
        .as_deref()
        .map(obj2py)
        .ok_or_else(|| {
            vm.new_value_error(format!("unknown object '{}'", args.txt.to_str().unwrap()))
        })
}

fn _ssl_nid2obj(nid: libc::c_int, vm: &VirtualMachine) -> PyResult<PyNid> {
    nid2obj(Nid::from_raw(nid))
        .as_deref()
        .map(obj2py)
        .ok_or_else(|| vm.new_value_error(format!("unknown NID {}", nid)))
}

fn _ssl_get_default_verify_paths() -> (String, String, String, String) {
    macro_rules! convert {
        ($f:ident) => {
            CStr::from_ptr(sys::$f()).to_string_lossy().into_owned()
        };
    }
    unsafe {
        (
            convert!(X509_get_default_cert_file_env),
            convert!(X509_get_default_cert_file),
            convert!(X509_get_default_cert_dir_env),
            convert!(X509_get_default_cert_dir),
        )
    }
}

fn _ssl_rand_status() -> i32 {
    unsafe { sys::RAND_status() }
}

fn _ssl_rand_add(string: Either<PyStrRef, PyBytesLike>, entropy: f64) {
    let f = |b: &[u8]| {
        for buf in b.chunks(libc::c_int::max_value() as usize) {
            unsafe { sys::RAND_add(buf.as_ptr() as *const _, buf.len() as _, entropy) }
        }
    };
    match string {
        Either::A(s) => f(s.borrow_value().as_bytes()),
        Either::B(b) => b.with_ref(f),
    }
}

fn _ssl_rand_bytes(n: i32, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    if n < 0 {
        return Err(vm.new_value_error("num must be positive".to_owned()));
    }
    let mut buf = vec![0; n as usize];
    openssl::rand::rand_bytes(&mut buf)
        .map(|()| buf)
        .map_err(|e| convert_openssl_error(vm, e))
}

fn _ssl_rand_pseudo_bytes(n: i32, vm: &VirtualMachine) -> PyResult<(Vec<u8>, bool)> {
    if n < 0 {
        return Err(vm.new_value_error("num must be positive".to_owned()));
    }
    let mut buf = vec![0; n as usize];
    let ret = unsafe { sys::RAND_pseudo_bytes(buf.as_mut_ptr(), n) };
    match ret {
        0 | 1 => Ok((buf, ret == 1)),
        _ => Err(convert_openssl_error(vm, ErrorStack::get())),
    }
}

#[pyclass(module = "ssl", name = "_SSLContext")]
struct PySslContext {
    ctx: PyRwLock<SslContextBuilder>,
    check_hostname: AtomicCell<bool>,
    protocol: SslVersion,
}

impl fmt::Debug for PySslContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("_SSLContext")
    }
}

impl PyValue for PySslContext {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

fn builder_as_ctx(x: &SslContextBuilder) -> &ssl::SslContextRef {
    unsafe { ssl::SslContextRef::from_ptr(x.as_ptr()) }
}

#[pyimpl(flags(BASETYPE))]
impl PySslContext {
    fn builder(&self) -> PyRwLockWriteGuard<'_, SslContextBuilder> {
        self.ctx.write()
    }
    fn exec_ctx<F, R>(&self, func: F) -> R
    where
        F: Fn(&ssl::SslContextRef) -> R,
    {
        let c = self.ctx.read();
        func(builder_as_ctx(&c))
    }

    #[pyslot]
    fn tp_new(cls: PyTypeRef, proto_version: i32, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let proto = SslVersion::try_from(proto_version)
            .map_err(|_| vm.new_value_error("invalid protocol version".to_owned()))?;
        let method = match proto {
            // SslVersion::Ssl3 => unsafe { ssl::SslMethod::from_ptr(sys::SSLv3_method()) },
            SslVersion::Tls => ssl::SslMethod::tls(),
            SslVersion::Tls1 => unsafe { ssl::SslMethod::from_ptr(sys::TLSv1_method()) },
            // TODO: Tls1_1, Tls1_2 ?
            SslVersion::TlsClient => ssl::SslMethod::tls_client(),
            SslVersion::TlsServer => ssl::SslMethod::tls_server(),
            _ => return Err(vm.new_value_error("invalid protocol version".to_owned())),
        };
        let mut builder =
            SslContextBuilder::new(method).map_err(|e| convert_openssl_error(vm, e))?;

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

        unsafe { sys::SSL_CTX_set_post_handshake_auth(builder.as_ptr(), 0) };

        builder
            .set_session_id_context(b"Python")
            .map_err(|e| convert_openssl_error(vm, e))?;

        PySslContext {
            ctx: PyRwLock::new(builder),
            check_hostname: AtomicCell::new(check_hostname),
            protocol: proto,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod]
    fn set_ciphers(&self, cipherlist: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let ciphers = cipherlist.borrow_value();
        if ciphers.contains('\0') {
            return Err(vm.new_value_error("embedded null character".to_owned()));
        }
        self.builder().set_cipher_list(ciphers).map_err(|_| {
            vm.new_exception_msg(ssl_error(vm), "No cipher can be selected.".to_owned())
        })
    }

    #[pyproperty]
    fn options(&self) -> libc::c_ulong {
        self.ctx.read().options().bits()
    }
    #[pyproperty(setter)]
    fn set_options(&self, opts: libc::c_ulong) {
        self.builder()
            .set_options(SslOptions::from_bits_truncate(opts));
    }
    #[pyproperty]
    fn protocol(&self) -> i32 {
        self.protocol as i32
    }
    #[pyproperty]
    fn verify_mode(&self) -> i32 {
        let mode = self.exec_ctx(|ctx| ctx.verify_mode());
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
    #[pyproperty(setter)]
    fn set_verify_mode(&self, cert: i32, vm: &VirtualMachine) -> PyResult<()> {
        let mut ctx = self.builder();
        let cert_req = CertRequirements::try_from(cert)
            .map_err(|_| vm.new_value_error("invalid value for verify_mode".to_owned()))?;
        let mode = match cert_req {
            CertRequirements::None if self.check_hostname.load() => {
                return Err(vm.new_value_error(
                    "Cannot set verify_mode to CERT_NONE when check_hostname is enabled."
                        .to_owned(),
                ))
            }
            CertRequirements::None => SslVerifyMode::NONE,
            CertRequirements::Optional => SslVerifyMode::PEER,
            CertRequirements::Required => SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT,
        };
        ctx.set_verify(mode);
        Ok(())
    }
    #[pyproperty]
    fn check_hostname(&self) -> bool {
        self.check_hostname.load()
    }
    #[pyproperty(setter)]
    fn set_check_hostname(&self, ch: bool) {
        let mut ctx = self.builder();
        if ch && builder_as_ctx(&ctx).verify_mode() == SslVerifyMode::NONE {
            ctx.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
        }
        self.check_hostname.store(ch);
    }

    #[pymethod]
    fn set_default_verify_paths(&self, vm: &VirtualMachine) -> PyResult<()> {
        self.builder()
            .set_default_verify_paths()
            .map_err(|e| convert_openssl_error(vm, e))
    }

    #[pymethod]
    fn _set_alpn_protocols(&self, protos: PyBytesLike, vm: &VirtualMachine) -> PyResult<()> {
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
                ssl::select_next_proto(&server, client).ok_or(ssl::AlpnError::NOACK)
            });
            Ok(())
        }
        #[cfg(not(ossl102))]
        {
            Err(vm.new_not_implemented_error(
                "The NPN extension requires OpenSSL 1.0.1 or later.".to_owned(),
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
            return Err(
                vm.new_type_error("cafile, capath and cadata cannot be all omitted".to_owned())
            );
        }

        if let Some(cadata) = args.cadata {
            let cert = match cadata {
                Either::A(s) => {
                    if !s.borrow_value().is_ascii() {
                        return Err(vm.new_type_error("Must be an ascii string".to_owned()));
                    }
                    X509::from_pem(s.borrow_value().as_bytes())
                }
                Either::B(b) => b.with_ref(X509::from_der),
            };
            let cert = cert.map_err(|e| convert_openssl_error(vm, e))?;
            let ret = self.exec_ctx(|ctx| {
                let store = ctx.cert_store();
                unsafe { sys::X509_STORE_add_cert(store.as_ptr(), cert.as_ptr()) }
            });
            if ret <= 0 {
                return Err(convert_openssl_error(vm, ErrorStack::get()));
            }
        }

        if args.cafile.is_some() || args.capath.is_some() {
            let ret = unsafe {
                let ctx = self.ctx.write();
                sys::SSL_CTX_load_verify_locations(
                    ctx.as_ptr(),
                    args.cafile
                        .as_ref()
                        .map_or_else(std::ptr::null, |cs| cs.as_ptr()),
                    args.capath
                        .as_ref()
                        .map_or_else(std::ptr::null, |cs| cs.as_ptr()),
                )
            };
            if ret != 1 {
                let errno = std::io::Error::last_os_error().raw_os_error().unwrap();
                let err = if errno != 0 {
                    super::os::errno_err(vm)
                } else {
                    convert_openssl_error(vm, ErrorStack::get())
                };
                return Err(err);
            }
        }

        Ok(())
    }

    #[pymethod]
    fn get_ca_certs(&self, binary_form: OptionalArg<bool>, vm: &VirtualMachine) -> PyResult {
        use openssl::stack::StackRef;
        let binary_form = binary_form.unwrap_or(false);
        let certs = unsafe {
            let stack =
                sys::X509_STORE_get0_objects(self.exec_ctx(|ctx| ctx.cert_store().as_ptr()));
            assert!(!stack.is_null());
            StackRef::<X509Object>::from_ptr(stack)
        };
        let certs = certs
            .iter()
            .filter_map(|cert| {
                let cert = cert.x509()?;
                Some(cert_to_py(vm, cert, binary_form))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(vm.ctx.new_list(certs))
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
            return Err(vm.new_not_implemented_error("password arg not yet supported".to_owned()));
        }
        let mut ctx = self.builder();
        ctx.set_certificate_chain_file(&certfile)
            .and_then(|()| {
                ctx.set_private_key_file(
                    keyfile.as_ref().unwrap_or(&certfile),
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
        let mut ssl = zelf
            .exec_ctx(|ctx| ssl::Ssl::new(ctx))
            .map_err(|e| convert_openssl_error(vm, e))?;

        let socket_type = if args.server_side {
            ssl.set_accept_state();
            SslServerOrClient::Server
        } else {
            ssl.set_connect_state();
            SslServerOrClient::Client
        };

        if let Some(hostname) = &args.server_hostname {
            let hostname = hostname.borrow_value();
            if hostname.is_empty() || hostname.starts_with('.') {
                return Err(vm.new_value_error(
                    "server_hostname cannot be an empty string or start with a leading dot."
                        .to_owned(),
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

        let stream = ssl::SslStream::new(ssl, args.sock.clone())
            .map_err(|e| convert_openssl_error(vm, e))?;

        // TODO: use this
        let _ = args.session;

        Ok(PySslSocket {
            ctx: zelf,
            stream: PyRwLock::new(stream),
            socket_type,
            server_hostname: args.server_hostname,
            owner: PyRwLock::new(args.owner.as_ref().map(PyWeak::downgrade)),
        })
    }
}

#[derive(FromArgs)]
// #[allow(dead_code)]
struct WrapSocketArgs {
    #[pyarg(any)]
    sock: PySocketRef,
    #[pyarg(any)]
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
    cafile: Option<CString>,
    #[pyarg(any, default)]
    capath: Option<CString>,
    #[pyarg(any, default)]
    cadata: Option<Either<PyStrRef, PyBytesLike>>,
}

#[derive(FromArgs)]
struct LoadCertChainArgs {
    #[pyarg(any)]
    certfile: PyPathLike,
    #[pyarg(any, optional)]
    keyfile: Option<PyPathLike>,
    #[pyarg(any, optional)]
    password: Option<Either<PyStrRef, PyCallable>>,
}

#[pyclass(module = "ssl", name = "_SSLSocket")]
struct PySslSocket {
    ctx: PyRef<PySslContext>,
    stream: PyRwLock<ssl::SslStream<PySocketRef>>,
    socket_type: SslServerOrClient,
    server_hostname: Option<PyStrRef>,
    owner: PyRwLock<Option<PyWeak>>,
}

impl fmt::Debug for PySslSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("_SSLSocket")
    }
}

impl PyValue for PySslSocket {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl PySslSocket {
    #[pyproperty]
    fn owner(&self) -> Option<PyObjectRef> {
        self.owner.read().as_ref().and_then(PyWeak::upgrade)
    }
    #[pyproperty(setter)]
    fn set_owner(&self, owner: PyObjectRef) {
        *self.owner.write() = Some(PyWeak::downgrade(&owner))
    }
    #[pyproperty]
    fn server_side(&self) -> bool {
        self.socket_type == SslServerOrClient::Server
    }
    #[pyproperty]
    fn context(&self) -> PyRef<PySslContext> {
        self.ctx.clone()
    }
    #[pyproperty]
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
            return Err(vm.new_value_error("handshake not done yet".to_owned()));
        }
        stream
            .ssl()
            .peer_certificate()
            .map(|cert| cert_to_py(vm, &cert, binary))
            .transpose()
    }

    #[pymethod]
    fn version(&self) -> Option<&'static str> {
        let v = self.stream.read().ssl().version_str();
        if v == "unknown" {
            None
        } else {
            Some(v)
        }
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
    fn compression(&self) -> Option<&'static str> {
        #[cfg(osslconf = "OPENSSL_NO_COMP")]
        {
            None
        }
        #[cfg(not(osslconf = "OPENSSL_NO_COMP"))]
        {
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
    }

    #[pymethod]
    fn do_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
        let mut stream = self.stream.write();
        let timeout = stream.get_ref().timeout.load();
        let deadline = if timeout > 0.0 {
            Some((std::time::Duration::from_secs_f64(timeout), Instant::now()))
        } else {
            None
        };
        let err = loop {
            let err = match stream.do_handshake() {
                Ok(()) => return Ok(()),
                Err(e) => e,
            };
            match err.code() {
                ssl::ErrorCode::WANT_READ | ssl::ErrorCode::WANT_WRITE => {
                    if let Some((timeout, ref start)) = deadline {
                        if start.elapsed() >= timeout {
                            let socket_timeout = vm.class("_socket", "timeout");
                            return Err(vm.new_exception_msg(
                                socket_timeout,
                                "The handshake operation timed out".to_owned(),
                            ));
                        }
                    } else if timeout == 0.0 {
                        // socket's non-blocking, we tried once and now it needs more to read/write
                        break err;
                    }
                    continue; // keep blocking
                }
                _ => break err,
            }
        };
        Err(convert_ssl_error(vm, err))
    }

    #[pymethod]
    fn write(&self, data: PyBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
        let mut stream = self.stream.write();
        data.with_ref(|b| stream.ssl_write(b))
            .map_err(|e| convert_ssl_error(vm, e))
    }

    #[pymethod]
    fn read(&self, n: usize, buffer: OptionalArg<PyRwBytesLike>, vm: &VirtualMachine) -> PyResult {
        let mut stream = self.stream.write();
        let ret_nread = buffer.is_present();
        let ssl_res = if let OptionalArg::Present(buffer) = buffer {
            buffer.with_ref(|buf| stream.ssl_read(buf).map(|n| vm.ctx.new_int(n)))
        } else {
            let mut buf = vec![0u8; n];
            stream.ssl_read(&mut buf).map(|n| {
                buf.truncate(n);
                vm.ctx.new_bytes(buf)
            })
        };
        ssl_res.or_else(|e| {
            if e.code() == ssl::ErrorCode::ZERO_RETURN
                && stream.get_shutdown() == ssl::ShutdownState::RECEIVED
            {
                Ok(if ret_nread {
                    vm.ctx.new_int(0)
                } else {
                    vm.ctx.new_bytes(vec![])
                })
            } else {
                Err(convert_ssl_error(vm, e))
            }
        })

        // .map_err(|e| convert_ssl_error(vm, e))?;
    }
}

fn ssl_error(vm: &VirtualMachine) -> PyTypeRef {
    vm.class("_ssl", "SSLError")
}

fn convert_openssl_error(vm: &VirtualMachine, err: ErrorStack) -> PyBaseExceptionRef {
    let cls = ssl_error(vm);
    match err.errors().last() {
        Some(e) => {
            // TODO: map the error codes to code names, e.g. "CERTIFICATE_VERIFY_FAILED", just requires a big hashmap/dict
            let errstr = e.reason().unwrap_or("unknown error");
            let msg = format!("{} (_ssl.c:{})", errstr, e.line());
            let reason = sys::ERR_GET_REASON(e.code());
            vm.new_exception(cls, vec![vm.ctx.new_int(reason), vm.ctx.new_str(msg)])
        }
        None => vm.new_exception_empty(cls),
    }
}
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
            Some(io_err) => return io_err.into_pyexception(vm),
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

type CipherTuple = (&'static str, &'static str, i32);

fn cipher_to_tuple(cipher: &ssl::SslCipherRef) -> CipherTuple {
    (cipher.name(), cipher.version(), cipher.bits().secret)
}

fn cert_to_py(vm: &VirtualMachine, cert: &X509Ref, binary: bool) -> PyResult {
    if binary {
        cert.to_der()
            .map(|b| vm.ctx.new_bytes(b))
            .map_err(|e| convert_openssl_error(vm, e))
    } else {
        let dict = vm.ctx.new_dict();

        let name_to_py = |name: &x509::X509NameRef| {
            name.entries()
                .map(|entry| {
                    let txt = obj2txt(entry.object(), false).into_pyobject(vm);
                    let data = vm.ctx.new_str(entry.data().as_utf8()?.to_owned());
                    Ok(vm.ctx.new_tuple(vec![vm.ctx.new_tuple(vec![txt, data])]))
                })
                .collect::<Result<_, _>>()
                .map(|list| vm.ctx.new_tuple(list))
                .map_err(|e| convert_openssl_error(vm, e))
        };

        dict.set_item("subject", name_to_py(cert.subject_name())?, vm)?;
        dict.set_item("issuer", name_to_py(cert.issuer_name())?, vm)?;

        let version = unsafe { sys::X509_get_version(cert.as_ptr()) };
        dict.set_item("version", vm.ctx.new_int(version), vm)?;

        let serial_num = cert
            .serial_number()
            .to_bn()
            .and_then(|bn| bn.to_hex_str())
            .map_err(|e| convert_openssl_error(vm, e))?;
        dict.set_item("serialNumber", vm.ctx.new_str(serial_num.to_owned()), vm)?;

        dict.set_item(
            "notBefore",
            vm.ctx.new_str(cert.not_before().to_string()),
            vm,
        )?;
        dict.set_item("notAfter", vm.ctx.new_str(cert.not_after().to_string()), vm)?;

        if let Some(names) = cert.subject_alt_names() {
            let san = names
                .iter()
                .filter_map(|gen_name| {
                    if let Some(email) = gen_name.email() {
                        Some(
                            vm.ctx
                                .new_tuple(vec![vm.ctx.new_str("email"), vm.ctx.new_str(email)]),
                        )
                    } else if let Some(dnsname) = gen_name.dnsname() {
                        Some(
                            vm.ctx
                                .new_tuple(vec![vm.ctx.new_str("DNS"), vm.ctx.new_str(dnsname)]),
                        )
                    } else if let Some(ip) = gen_name.ipaddress() {
                        Some(vm.ctx.new_tuple(vec![
                            vm.ctx.new_str("IP Address"),
                            vm.ctx.new_str(String::from_utf8_lossy(ip).into_owned()),
                        ]))
                    } else {
                        // TODO: convert every type of general name:
                        // https://github.com/python/cpython/blob/3.6/Modules/_ssl.c#L1092-L1231
                        None
                    }
                })
                .collect();
            dict.set_item("subjectAltName", vm.ctx.new_tuple(san), vm)?;
        };

        Ok(dict.into_object())
    }
}

#[allow(non_snake_case)]
fn _ssl__test_decode_cert(path: PyPathLike, vm: &VirtualMachine) -> PyResult {
    let pem = std::fs::read(&path).map_err(|e| e.into_pyexception(vm))?;
    let x509 = X509::from_pem(&pem).map_err(|e| convert_openssl_error(vm, e))?;
    cert_to_py(vm, &x509, false)
}

fn parse_version_info(mut n: i64) -> (u8, u8, u8, u8, u8) {
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    // if openssl is vendored, it doesn't know the locations of system certificates
    match option_env!("OPENSSL_NO_VENDOR") {
        None | Some("0") => {}
        _ => openssl_probe::init_ssl_cert_env_vars(),
    }
    openssl::init();

    // the openssl version from the API headers
    let openssl_api_version = i64::from_str_radix(env!("OPENSSL_API_VERSION"), 16).unwrap();

    let ctx = &vm.ctx;

    let ssl_error = create_exception_type("SSLError", &vm.ctx.exceptions.os_error);

    let ssl_cert_verification_error = pytype::new(
        ctx.types.type_type.clone(),
        "SSLCertVerificationError",
        ssl_error.clone(),
        vec![ssl_error.clone(), ctx.exceptions.value_error.clone()],
        Default::default(),
        crate::exceptions::exception_slots(),
    )
    .unwrap();
    let ssl_zero_return_error = create_exception_type("SSLZeroReturnError", &ssl_error);
    let ssl_want_read_error = create_exception_type("SSLWantReadError", &ssl_error);
    let ssl_want_write_error = create_exception_type("SSLWantWriteError", &ssl_error);
    let ssl_syscall_error = create_exception_type("SSLSyscallError", &ssl_error);
    let ssl_eof_error = create_exception_type("SSLEOFError", &ssl_error);

    let module = py_module!(vm, "_ssl", {
        "_SSLContext" => PySslContext::make_class(ctx),
        "_SSLSocket" => PySslSocket::make_class(ctx),
        "SSLError" => ssl_error,
        "SSLCertVerificationError" => ssl_cert_verification_error,
        "SSLZeroReturnError" => ssl_zero_return_error,
        "SSLWantReadError" => ssl_want_read_error,
        "SSLWantWriteError" => ssl_want_write_error,
        "SSLSyscallError" => ssl_syscall_error,
        "SSLEOFError" => ssl_eof_error,
        "txt2obj" => named_function!(ctx, _ssl, txt2obj),
        "nid2obj" => named_function!(ctx, _ssl, nid2obj),
        "get_default_verify_paths" => named_function!(ctx, _ssl, get_default_verify_paths),
        "RAND_status" => named_function!(ctx, _ssl, rand_status),
        "RAND_add" => named_function!(ctx, _ssl, rand_add),
        "RAND_bytes" => named_function!(ctx, _ssl, rand_bytes),
        "RAND_pseudo_bytes" => named_function!(ctx, _ssl, rand_pseudo_bytes),
        "_test_decode_cert" => named_function!(ctx, _ssl, _test_decode_cert),

        // Constants
        "OPENSSL_VERSION" => ctx.new_str(openssl::version::version().to_owned()),
        "OPENSSL_VERSION_NUMBER" => ctx.new_int(openssl::version::number()),
        "OPENSSL_VERSION_INFO" => parse_version_info(openssl::version::number()).into_pyobject(vm),
        "_OPENSSL_API_VERSION" => parse_version_info(openssl_api_version).into_pyobject(vm),
        "_DEFAULT_CIPHERS" => ctx.new_str(DEFAULT_CIPHER_STRING),
        // "PROTOCOL_SSLv2" => ctx.new_int(SslVersion::Ssl2 as u32), unsupported
        // "PROTOCOL_SSLv3" => ctx.new_int(SslVersion::Ssl3 as u32),
        "PROTOCOL_SSLv23" => ctx.new_int(SslVersion::Tls as u32),
        "PROTOCOL_TLS" => ctx.new_int(SslVersion::Tls as u32),
        "PROTOCOL_TLS_CLIENT" => ctx.new_int(SslVersion::TlsClient as u32),
        "PROTOCOL_TLS_SERVER" => ctx.new_int(SslVersion::TlsServer as u32),
        "PROTOCOL_TLSv1" => ctx.new_int(SslVersion::Tls1 as u32),
        "PROTO_MINIMUM_SUPPORTED" => ctx.new_int(ProtoVersion::MinSupported as i32),
        "PROTO_SSLv3" => ctx.new_int(ProtoVersion::Ssl3 as i32),
        "PROTO_TLSv1" => ctx.new_int(ProtoVersion::Tls1 as i32),
        "PROTO_TLSv1_1" => ctx.new_int(ProtoVersion::Tls1_1 as i32),
        "PROTO_TLSv1_2" => ctx.new_int(ProtoVersion::Tls1_2 as i32),
        "PROTO_TLSv1_3" => ctx.new_int(ProtoVersion::Tls1_3 as i32),
        "PROTO_MAXIMUM_SUPPORTED" => ctx.new_int(ProtoVersion::MaxSupported as i32),
        "OP_ALL" => ctx.new_int(sys::SSL_OP_ALL & !sys::SSL_OP_DONT_INSERT_EMPTY_FRAGMENTS),
        "OP_NO_SSLv2" => ctx.new_int(sys::SSL_OP_NO_SSLv2),
        "OP_NO_SSLv3" => ctx.new_int(sys::SSL_OP_NO_SSLv3),
        "OP_NO_TLSv1" => ctx.new_int(sys::SSL_OP_NO_TLSv1),
        "OP_NO_TLSv1_3" => ctx.new_int(sys::SSL_OP_NO_TLSv1_3),
        "OP_CIPHER_SERVER_PREFERENCE" => ctx.new_int(sys::SSL_OP_CIPHER_SERVER_PREFERENCE),
        "OP_SINGLE_DH_USE" => ctx.new_int(sys::SSL_OP_SINGLE_DH_USE),
        "OP_NO_TICKET" => ctx.new_int(sys::SSL_OP_NO_TICKET),
        // #ifdef SSL_OP_SINGLE_ECDH_USE
        // "OP_SINGLE_ECDH_USE" => ctx.new_int(sys::SSL_OP_SINGLE_ECDH_USE),
        // #endif
        "HAS_TLS_UNIQUE" => ctx.new_bool(true),
        "CERT_NONE" => ctx.new_int(CertRequirements::None as u32),
        "CERT_OPTIONAL" => ctx.new_int(CertRequirements::Optional as u32),
        "CERT_REQUIRED" => ctx.new_int(CertRequirements::Required as u32),
        "VERIFY_DEFAULT" => ctx.new_int(0),
        // "VERIFY_CRL_CHECK_LEAF" => sys::X509_V_FLAG_CRL_CHECK,
        // "VERIFY_CRL_CHECK_CHAIN" => sys::X509_V_FLAG_CRL_CHECK|sys::X509_V_FLAG_CRL_CHECK_ALL,
        // "VERIFY_X509_STRICT" => X509_V_FLAG_X509_STRICT,
        "SSL_ERROR_ZERO_RETURN" => ctx.new_int(sys::SSL_ERROR_ZERO_RETURN),
        "SSL_ERROR_WANT_READ" => ctx.new_int(sys::SSL_ERROR_WANT_READ),
        "SSL_ERROR_WANT_WRITE" => ctx.new_int(sys::SSL_ERROR_WANT_WRITE),
        // "SSL_ERROR_WANT_X509_LOOKUP" => ctx.new_int(sys::SSL_ERROR_WANT_X509_LOOKUP),
        "SSL_ERROR_SYSCALL" => ctx.new_int(sys::SSL_ERROR_SYSCALL),
        "SSL_ERROR_SSL" => ctx.new_int(sys::SSL_ERROR_SSL),
        "SSL_ERROR_WANT_CONNECT" => ctx.new_int(sys::SSL_ERROR_WANT_CONNECT),
        "SSL_ERROR_EOF" => ctx.new_int(8), // custom for python
        // "SSL_ERROR_INVALID_ERROR_CODE" => ctx.new_int(sys::SSL_ERROR_INVALID_ERROR_CODE),
        // TODO: so many more of these
        "ALERT_DESCRIPTION_DECODE_ERROR" => ctx.new_int(sys::SSL_AD_DECODE_ERROR),
        "ALERT_DESCRIPTION_ILLEGAL_PARAMETER" => ctx.new_int(sys::SSL_AD_ILLEGAL_PARAMETER),
        "ALERT_DESCRIPTION_UNRECOGNIZED_NAME" => ctx.new_int(sys::SSL_AD_UNRECOGNIZED_NAME),

        "HAS_SNI" => ctx.new_bool(true),
        "HAS_ECDH" => ctx.new_bool(false),
        "HAS_NPN" => ctx.new_bool(false),
        "HAS_ALPN" => ctx.new_bool(true),
        "HAS_SSLv2" => ctx.new_bool(true),
        "HAS_SSLv3" => ctx.new_bool(true),
        "HAS_TLSv1" => ctx.new_bool(true),
        "HAS_TLSv1_1" => ctx.new_bool(true),
        "HAS_TLSv1_2" => ctx.new_bool(true),
        "HAS_TLSv1_3" => ctx.new_bool(cfg!(ossl111)),
    });

    #[cfg(ossl101)]
    extend_module!(vm, module, {
        "OP_NO_COMPRESSION" => ctx.new_int(sys::SSL_OP_NO_COMPRESSION),
        "OP_NO_TLSv1_1" => ctx.new_int(sys::SSL_OP_NO_TLSv1_1),
        "OP_NO_TLSv1_2" => ctx.new_int(sys::SSL_OP_NO_TLSv1_2),
    });

    extend_module_platform_specific(&module, vm);

    module
}

#[cfg(windows)]
fn extend_module_platform_specific(module: &PyObjectRef, vm: &VirtualMachine) {
    let ctx = &vm.ctx;
    extend_module!(vm, module, {
        "enum_certificates" => named_function!(ctx, _ssl, enum_certificates),
    })
}

#[cfg(not(windows))]
fn extend_module_platform_specific(_module: &PyObjectRef, _vm: &VirtualMachine) {}
