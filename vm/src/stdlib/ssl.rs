use super::socket::PySocketRef;
use crate::byteslike::PyBytesLike;
use crate::common::cell::{PyRwLock, PyRwLockWriteGuard};
use crate::exceptions::{IntoPyException, PyBaseExceptionRef};
use crate::function::OptionalArg;
use crate::obj::objbytearray::PyByteArrayRef;
use crate::obj::objstr::PyStringRef;
use crate::obj::{objtype::PyClassRef, objweakref::PyWeak};
use crate::pyobject::{
    BorrowValue, Either, IntoPyObject, ItemProtocol, PyClassImpl, PyObjectRef, PyRef, PyResult,
    PyValue,
};
use crate::types::create_type;
use crate::VirtualMachine;

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
    }
}

#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, PartialEq)]
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
fn ssl_enum_certificates(store_name: PyStringRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    use crate::obj::objset::PyFrozenSet;
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
    #[pyarg(positional_or_keyword)]
    txt: CString,
    #[pyarg(positional_or_keyword, default = "false")]
    name: bool,
}
fn ssl_txt2obj(args: Txt2ObjArgs, vm: &VirtualMachine) -> PyResult<PyNid> {
    txt2obj(&args.txt, !args.name)
        .as_deref()
        .map(obj2py)
        .ok_or_else(|| {
            vm.new_value_error(format!("unknown object '{}'", args.txt.to_str().unwrap()))
        })
}

fn ssl_nid2obj(nid: libc::c_int, vm: &VirtualMachine) -> PyResult<PyNid> {
    nid2obj(Nid::from_raw(nid))
        .as_deref()
        .map(obj2py)
        .ok_or_else(|| vm.new_value_error(format!("unknown NID {}", nid)))
}

fn ssl_get_default_verify_paths() -> (String, String, String, String) {
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

fn ssl_rand_status() -> i32 {
    unsafe { sys::RAND_status() }
}

fn ssl_rand_add(string: Either<PyStringRef, PyBytesLike>, entropy: f64) {
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

fn ssl_rand_bytes(n: i32, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    if n < 0 {
        return Err(vm.new_value_error("num must be positive".to_owned()));
    }
    let mut buf = vec![0; n as usize];
    openssl::rand::rand_bytes(&mut buf)
        .map(|()| buf)
        .map_err(|e| convert_openssl_error(vm, e))
}

fn ssl_rand_pseudo_bytes(n: i32, vm: &VirtualMachine) -> PyResult<(Vec<u8>, bool)> {
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

#[pyclass(name = "_SSLContext")]
struct PySslContext {
    ctx: PyRwLock<SslContextBuilder>,
    check_hostname: bool,
}

impl fmt::Debug for PySslContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("_SSLContext")
    }
}

impl PyValue for PySslContext {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_ssl", "_SSLContext")
    }
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
        func(unsafe { &**(&*c as *const SslContextBuilder as *const ssl::SslContext) })
    }
    fn ptr(&self) -> *mut sys::SSL_CTX {
        (*self.ctx.write()).as_ptr()
    }

    #[pyslot]
    fn tp_new(cls: PyClassRef, proto_version: i32, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let proto = SslVersion::try_from(proto_version)
            .map_err(|_| vm.new_value_error("invalid protocol version".to_owned()))?;
        let method = match proto {
            SslVersion::Ssl2 => todo!(),
            SslVersion::Ssl3 => todo!(),
            SslVersion::Tls => ssl::SslMethod::tls(),
            SslVersion::Tls1 => todo!(),
            // TODO: Tls1_1, Tls1_2 ?
            SslVersion::TlsClient => ssl::SslMethod::tls_client(),
            SslVersion::TlsServer => ssl::SslMethod::tls_server(),
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
            check_hostname,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod]
    fn set_ciphers(&self, cipherlist: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
        let ciphers = cipherlist.borrow_value();
        if ciphers.contains('\0') {
            return Err(vm.new_value_error("embedded null character".to_owned()));
        }
        self.builder().set_cipher_list(ciphers).map_err(|_| {
            vm.new_exception_msg(ssl_error(vm), "No cipher can be selected.".to_owned())
        })
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
        let cert_req = CertRequirements::try_from(cert)
            .map_err(|_| vm.new_value_error("invalid value for verify_mode".to_owned()))?;
        let mode = match cert_req {
            CertRequirements::None if self.check_hostname => {
                return Err(vm.new_value_error(
                    "Cannot set verify_mode to CERT_NONE when check_hostname is enabled."
                        .to_owned(),
                ))
            }
            CertRequirements::None => SslVerifyMode::NONE,
            CertRequirements::Optional => SslVerifyMode::PEER,
            CertRequirements::Required => SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT,
        };
        self.builder().set_verify(mode);
        Ok(())
    }

    #[pymethod]
    fn set_default_verify_paths(&self, vm: &VirtualMachine) -> PyResult<()> {
        self.builder()
            .set_default_verify_paths()
            .map_err(|e| convert_openssl_error(vm, e))
    }

    #[pymethod]
    fn load_verify_locations(
        &self,
        args: LoadVerifyLocationsArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if args.cafile.is_none() && args.capath.is_none() && args.cadata.is_none() {
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
                sys::SSL_CTX_load_verify_locations(
                    self.ptr(),
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
    fn _wrap_socket(
        zelf: PyRef<Self>,
        args: WrapSocketArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PySslSocket> {
        let ssl = {
            let ptr = zelf.ptr();
            let ctx = unsafe { ssl::SslContext::from_ptr(ptr) };
            let ssl = ssl::Ssl::new(&ctx).map_err(|e| convert_openssl_error(vm, e))?;
            std::mem::forget(ctx);
            ssl
        };

        let mut stream = ssl::SslStreamBuilder::new(ssl, args.sock.clone());

        let socket_type = if args.server_side {
            stream.set_accept_state();
            SslServerOrClient::Server
        } else {
            stream.set_connect_state();
            SslServerOrClient::Client
        };

        // TODO: use this
        let _ = args.session;

        Ok(PySslSocket {
            ctx: zelf,
            stream: PyRwLock::new(Some(stream)),
            socket_type,
            server_hostname: args.server_hostname,
            owner: PyRwLock::new(args.owner.as_ref().map(PyWeak::downgrade)),
        })
    }
}

#[derive(FromArgs)]
// #[allow(dead_code)]
struct WrapSocketArgs {
    #[pyarg(positional_or_keyword)]
    sock: PySocketRef,
    #[pyarg(positional_or_keyword)]
    server_side: bool,
    #[pyarg(positional_or_keyword, default = "None")]
    server_hostname: Option<PyStringRef>,
    #[pyarg(keyword_only, default = "None")]
    owner: Option<PyObjectRef>,
    #[pyarg(keyword_only, default = "None")]
    session: Option<PyObjectRef>,
}

#[derive(FromArgs)]
struct LoadVerifyLocationsArgs {
    #[pyarg(positional_or_keyword, default = "None")]
    cafile: Option<CString>,
    #[pyarg(positional_or_keyword, default = "None")]
    capath: Option<CString>,
    #[pyarg(positional_or_keyword, default = "None")]
    cadata: Option<Either<PyStringRef, PyBytesLike>>,
}

#[pyclass(name = "_SSLSocket")]
struct PySslSocket {
    ctx: PyRef<PySslContext>,
    stream: PyRwLock<Option<ssl::SslStreamBuilder<PySocketRef>>>,
    socket_type: SslServerOrClient,
    server_hostname: Option<PyStringRef>,
    owner: PyRwLock<Option<PyWeak>>,
}

impl fmt::Debug for PySslSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("_SSLSocket")
    }
}

impl PyValue for PySslSocket {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_ssl", "_SSLSocket")
    }
}

#[pyimpl]
impl PySslSocket {
    fn stream_builder(&self) -> ssl::SslStreamBuilder<PySocketRef> {
        std::mem::replace(&mut *self.stream.write(), None).unwrap()
    }
    fn exec_stream<F, R>(&self, func: F) -> R
    where
        F: Fn(&mut ssl::SslStream<PySocketRef>) -> R,
    {
        let mut b = self.stream.write();
        func(unsafe {
            &mut *(b.as_mut().unwrap() as *mut ssl::SslStreamBuilder<_> as *mut ssl::SslStream<_>)
        })
    }
    fn set_stream(&self, stream: ssl::SslStream<PySocketRef>) {
        *self.stream.write() = Some(unsafe { std::mem::transmute(stream) });
    }

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
    fn server_hostname(&self) -> Option<PyStringRef> {
        self.server_hostname.clone()
    }

    #[pymethod]
    fn peer_certificate(
        &self,
        binary: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let binary = binary.unwrap_or(false);
        if !self.exec_stream(|stream| stream.ssl().is_init_finished()) {
            return Err(vm.new_value_error("handshake not done yet".to_owned()));
        }
        self.exec_stream(|stream| stream.ssl().peer_certificate())
            .map(|cert| cert_to_py(vm, &cert, binary))
            .transpose()
    }

    #[pymethod]
    fn do_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
        // Either a stream builder or a mid-handshake stream from WANT_READ or WANT_WRITE
        let mut handshaker: Either<_, ssl::MidHandshakeSslStream<_>> =
            Either::A(self.stream_builder());
        loop {
            let handshake_result = match handshaker {
                Either::A(s) => s.handshake(),
                Either::B(s) => s.handshake(),
            };
            match handshake_result {
                Ok(stream) => {
                    self.set_stream(stream);
                    return Ok(());
                }
                Err(ssl::HandshakeError::SetupFailure(e)) => {
                    return Err(convert_openssl_error(vm, e))
                }
                Err(ssl::HandshakeError::WouldBlock(s)) => handshaker = Either::B(s),
                Err(ssl::HandshakeError::Failure(s)) => {
                    return Err(convert_ssl_error(vm, s.into_error()))
                }
            }
        }
    }

    #[pymethod]
    fn write(&self, data: PyBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
        data.with_ref(|b| self.exec_stream(|stream| stream.ssl_write(b)))
            .map_err(|e| convert_ssl_error(vm, e))
    }

    #[pymethod]
    fn read(&self, n: usize, buffer: OptionalArg<PyByteArrayRef>, vm: &VirtualMachine) -> PyResult {
        if let OptionalArg::Present(buffer) = buffer {
            let n = self
                .exec_stream(|stream| {
                    let mut buf = buffer.borrow_value_mut();
                    stream.ssl_read(&mut buf.elements)
                })
                .map_err(|e| convert_ssl_error(vm, e))?;
            Ok(vm.ctx.new_int(n))
        } else {
            let mut buf = vec![0u8; n];
            buf.truncate(n);
            Ok(vm.ctx.new_bytes(buf))
        }
    }
}

fn ssl_error(vm: &VirtualMachine) -> PyClassRef {
    vm.class("_ssl", "SSLError")
}

fn convert_openssl_error(vm: &VirtualMachine, err: ErrorStack) -> PyBaseExceptionRef {
    let cls = ssl_error(vm);
    match err.errors().first() {
        Some(e) => {
            // let no = "unknown";
            // let msg = format!(
            //     "openssl error code {}, from library {}, in function {}, on line {}, with reason {}, and extra data {}",
            //     e.code(), e.library().unwrap_or(no), e.function().unwrap_or(no), e.line(),
            //     e.reason().unwrap_or(no), e.data().unwrap_or("none"),
            // );
            // TODO: map the error codes to code names, e.g. "CERTIFICATE_VERIFY_FAILED", just requires a big hashmap/dict
            let msg = e.to_string();
            vm.new_exception_msg(cls, msg)
        }
        None => vm.new_exception_empty(cls),
    }
}
fn convert_ssl_error(vm: &VirtualMachine, e: ssl::Error) -> PyBaseExceptionRef {
    match e.into_io_error() {
        Ok(io_err) => io_err.into_pyexception(vm),
        Err(e) => convert_openssl_error(vm, e.ssl_error().unwrap().clone()),
    }
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
                    let txt = match obj2txt(entry.object(), false) {
                        Some(s) => vm.ctx.new_str(s),
                        None => vm.get_none(),
                    };
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
    let ctx = &vm.ctx;
    let ssl_error = create_type(
        "SSLError",
        &vm.ctx.types.type_type,
        &vm.ctx.exceptions.os_error,
    );
    let module = py_module!(vm, "_ssl", {
        "_SSLContext" => PySslContext::make_class(ctx),
        "_SSLSocket" => PySslSocket::make_class(ctx),
        "SSLError" => ssl_error,
        "txt2obj" => ctx.new_function(ssl_txt2obj),
        "nid2obj" => ctx.new_function(ssl_nid2obj),
        "get_default_verify_paths" => ctx.new_function(ssl_get_default_verify_paths),
        "RAND_status" => ctx.new_function(ssl_rand_status),
        "RAND_add" => ctx.new_function(ssl_rand_add),
        "RAND_bytes" => ctx.new_function(ssl_rand_bytes),
        "RAND_pseudo_bytes" => ctx.new_function(ssl_rand_pseudo_bytes),

        // Constants
        "OPENSSL_VERSION" => ctx.new_str(openssl::version::version().to_owned()),
        "OPENSSL_VERSION_NUMBER" => ctx.new_int(openssl::version::number()),
        "OPENSSL_VERSION_INFO" => parse_version_info(openssl::version::number()).into_pyobject(vm),
        "PROTOCOL_SSLv2" => ctx.new_int(SslVersion::Ssl2 as u32),
        "PROTOCOL_SSLv3" => ctx.new_int(SslVersion::Ssl3 as u32),
        "PROTOCOL_SSLv23" => ctx.new_int(SslVersion::Tls as u32),
        "PROTOCOL_TLS" => ctx.new_int(SslVersion::Tls as u32),
        "PROTOCOL_TLS_CLIENT" => ctx.new_int(SslVersion::TlsClient as u32),
        "PROTOCOL_TLS_SERVER" => ctx.new_int(SslVersion::TlsServer as u32),
        "PROTOCOL_TLSv1" => ctx.new_int(SslVersion::Tls1 as u32),
        "OP_NO_SSLv2" => ctx.new_int(sys::SSL_OP_NO_SSLv2),
        "OP_NO_SSLv3" => ctx.new_int(sys::SSL_OP_NO_SSLv3),
        "OP_NO_TLSv1" => ctx.new_int(sys::SSL_OP_NO_TLSv1),
        // "OP_NO_TLSv1_1" => ctx.new_int(sys::SSL_OP_NO_TLSv1_1),
        // "OP_NO_TLSv1_2" => ctx.new_int(sys::SSL_OP_NO_TLSv1_2),
        "OP_NO_TLSv1_3" => ctx.new_int(sys::SSL_OP_NO_TLSv1_3),
        "OP_CIPHER_SERVER_PREFERENCE" => ctx.new_int(sys::SSL_OP_CIPHER_SERVER_PREFERENCE),
        "OP_SINGLE_DH_USE" => ctx.new_int(sys::SSL_OP_SINGLE_DH_USE),
        "OP_NO_TICKET" => ctx.new_int(sys::SSL_OP_NO_TICKET),
        // #ifdef SSL_OP_SINGLE_ECDH_USE
        // "OP_SINGLE_ECDH_USE" => ctx.new_int(sys::SSL_OP_SINGLE_ECDH_USE),
        // #endif
        // #ifdef SSL_OP_NO_COMPRESSION
        // "OP_NO_COMPRESSION" => ctx.new_int(sys::SSL_OP_NO_COMPRESSION),
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
        // "SSL_ERROR_EOF" => ctx.new_int(sys::SSL_ERROR_EOF),
        // "SSL_ERROR_INVALID_ERROR_CODE" => ctx.new_int(sys::SSL_ERROR_INVALID_ERROR_CODE),
        // TODO: so many more of these
        "ALERT_DESCRIPTION_DECODE_ERROR" => ctx.new_int(sys::SSL_AD_DECODE_ERROR),
        "ALERT_DESCRIPTION_ILLEGAL_PARAMETER" => ctx.new_int(sys::SSL_AD_ILLEGAL_PARAMETER),
        "ALERT_DESCRIPTION_UNRECOGNIZED_NAME" => ctx.new_int(sys::SSL_AD_UNRECOGNIZED_NAME),
    });

    extend_module_platform_specific(&module, vm);

    module
}

#[cfg(windows)]
fn extend_module_platform_specific(module: &PyObjectRef, vm: &VirtualMachine) {
    let ctx = &vm.ctx;
    extend_module!(vm, module, {
        "enum_certificates" => ctx.new_function(ssl_enum_certificates),
    })
}

#[cfg(not(windows))]
fn extend_module_platform_specific(_module: &PyObjectRef, _vm: &VirtualMachine) {}
