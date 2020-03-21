use super::socket::PySocketRef;
use crate::exceptions::PyBaseExceptionRef;
use crate::function::OptionalArg;
use crate::obj::objbytearray::PyByteArrayRef;
use crate::obj::objbyteinner::PyBytesLike;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::{objtype::PyClassRef, objweakref::PyWeak};
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject};
use crate::types::create_type;
use crate::VirtualMachine;

use std::cell::{RefCell, RefMut};
use std::convert::TryFrom;
use std::ffi::{CStr, CString};
use std::fmt;

use foreign_types_shared::{ForeignType, ForeignTypeRef};
use openssl::{
    asn1::{Asn1Object, Asn1ObjectRef},
    nid::Nid,
    ssl::{self, SslContextBuilder, SslVerifyMode},
};

mod sys {
    use libc::{c_char, c_int};
    pub use openssl_sys::*;
    extern "C" {
        pub fn OBJ_txt2obj(s: *const c_char, no_name: c_int) -> *mut ASN1_OBJECT;
        pub fn OBJ_nid2obj(n: c_int) -> *mut ASN1_OBJECT;
        pub fn TLS_server_method() -> *const SSL_METHOD;
        pub fn TLS_client_method() -> *const SSL_METHOD;
        pub fn SSL_CTX_get_verify_mode(ctx: *const SSL_CTX) -> c_int;
        pub fn X509_get_default_cert_file_env() -> *const c_char;
        pub fn X509_get_default_cert_file() -> *const c_char;
        pub fn X509_get_default_cert_dir_env() -> *const c_char;
        pub fn X509_get_default_cert_dir() -> *const c_char;
    }
}

#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
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

#[derive(Debug)]
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

#[pyclass(name = "_SSLContext")]
struct PySslContext {
    ctx: RefCell<SslContextBuilder>,
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
    fn builder(&self) -> RefMut<SslContextBuilder> {
        self.ctx.borrow_mut()
    }
    // fn ctx(&self) -> Ref<SslContextRef> {
    //     Ref::map(self.ctx.borrow(), |ctx| unsafe {
    //         SslContextRef::from_ptr(ctx.as_ptr())
    //     })
    // }
    fn ptr(&self) -> *mut sys::SSL_CTX {
        self.ctx.borrow().as_ptr()
    }

    #[pyslot]
    fn tp_new(cls: PyClassRef, proto_version: i32, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let proto = SslVersion::try_from(proto_version)
            .map_err(|_| vm.new_value_error("invalid protocol version".to_owned()))?;
        let method = match proto {
            SslVersion::Ssl2 => todo!(),
            SslVersion::Ssl3 => todo!(),
            SslVersion::Tls => unsafe { ssl::SslMethod::from_ptr(sys::TLS_method()) },
            SslVersion::Tls1 => todo!(),
            // TODO: Tls1_1, Tls1_2 ?
            SslVersion::TlsClient => unsafe { ssl::SslMethod::from_ptr(sys::TLS_client_method()) },
            SslVersion::TlsServer => unsafe { ssl::SslMethod::from_ptr(sys::TLS_server_method()) },
        };
        let mut builder =
            SslContextBuilder::new(method).map_err(|e| convert_openssl_error(vm, e))?;
        let check_hostname = matches!(proto, SslVersion::TlsClient);
        builder.set_verify(if check_hostname {
            SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT
        } else {
            SslVerifyMode::NONE
        });
        PySslContext {
            ctx: RefCell::new(builder),
            check_hostname,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod]
    fn set_ciphers(&self, cipherlist: CString, vm: &VirtualMachine) -> PyResult<()> {
        self.builder()
            .set_cipher_list(cipherlist.to_str().unwrap())
            .map_err(|_| {
                vm.new_exception_msg(ssl_error(vm), "No cipher can be selected.".to_owned())
            })
    }

    #[pyproperty]
    fn verify_mode(&self) -> i32 {
        let mode = unsafe { sys::SSL_CTX_get_verify_mode(self.ptr()) };
        let mode =
            SslVerifyMode::from_bits(mode).expect("bad SSL_CTX_get_verify_mode return value");
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

        if let Some(_cadata) = args.cadata {
            todo!()
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
                    convert_openssl_error(vm, openssl::error::ErrorStack::get())
                };
                return Err(err);
            }
        }

        Ok(())
    }

    #[pymethod]
    fn _wrap_socket(
        zelf: PyRef<Self>,
        args: WrapSocketArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PySslSocket> {
        let server_hostname = args
            .server_hostname
            .map(|s| {
                vm.encode(
                    s.into_object(),
                    Some(PyString::from("ascii").into_ref(vm)),
                    None,
                )
                .and_then(|res| PyBytesRef::try_from_object(vm, res))
            })
            .transpose()?;

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
            stream: RefCell::new(Some(stream)),
            socket_type,
            server_hostname,
            owner: RefCell::new(args.owner.as_ref().map(PyWeak::downgrade)),
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
    cadata: Option<PyStringRef>,
}

#[pyclass(name = "_SSLSocket")]
struct PySslSocket {
    ctx: PyRef<PySslContext>,
    stream: RefCell<Option<ssl::SslStreamBuilder<PySocketRef>>>,
    socket_type: SslServerOrClient,
    server_hostname: Option<PyBytesRef>,
    owner: RefCell<Option<PyWeak>>,
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
        self.stream.replace(None).unwrap()
    }
    fn stream(&self) -> RefMut<ssl::SslStream<PySocketRef>> {
        RefMut::map(self.stream.borrow_mut(), |b| {
            let b = b.as_mut().unwrap();
            unsafe { &mut *(b as *mut ssl::SslStreamBuilder<_> as *mut ssl::SslStream<_>) }
        })
    }
    fn set_stream(&self, stream: ssl::SslStream<PySocketRef>) {
        let prev = self
            .stream
            .replace(Some(unsafe { std::mem::transmute(stream) }));
        debug_assert!(prev.is_none());
    }

    #[pyproperty]
    fn owner(&self) -> Option<PyObjectRef> {
        self.owner.borrow().as_ref().and_then(PyWeak::upgrade)
    }
    #[pyproperty(setter)]
    fn set_owner(&self, owner: PyObjectRef) {
        *self.owner.borrow_mut() = Some(PyWeak::downgrade(&owner))
    }
    #[pyproperty]
    fn server_side(&self) -> bool {
        matches!(self.socket_type, SslServerOrClient::Server)
    }
    #[pyproperty]
    fn context(&self) -> PyRef<PySslContext> {
        self.ctx.clone()
    }
    #[pyproperty]
    fn server_hostname(&self) -> Option<PyBytesRef> {
        self.server_hostname.clone()
    }

    #[pymethod]
    fn do_handshake(&self, vm: &VirtualMachine) -> PyResult<()> {
        use crate::pyobject::Either;
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
        data.with_ref(|b| self.stream().ssl_write(b))
            .map_err(|e| convert_ssl_error(vm, e))
    }

    #[pymethod]
    fn read(&self, n: usize, buffer: OptionalArg<PyByteArrayRef>, vm: &VirtualMachine) -> PyResult {
        if let OptionalArg::Present(buffer) = buffer {
            let mut buf = buffer.borrow_value_mut();
            let n = self
                .stream()
                .ssl_read(&mut buf.elements)
                .map_err(|e| convert_ssl_error(vm, e))?;
            Ok(vm.new_int(n))
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

fn convert_openssl_error(
    vm: &VirtualMachine,
    err: openssl::error::ErrorStack,
) -> PyBaseExceptionRef {
    let cls = ssl_error(vm);
    match err.errors().first() {
        Some(e) => {
            let no = "unknown";
            let msg = format!(
                "openssl error code {}, from library {}, in function {}, on line {}, with reason {}, and extra data {}",
                e.code(), e.library().unwrap_or(no), e.function().unwrap_or(no), e.line(),
                e.reason().unwrap_or(no), e.data().unwrap_or("none"),
            );
            vm.new_exception_msg(cls, msg)
        }
        None => vm.new_exception_empty(cls),
    }
}
fn convert_ssl_error(vm: &VirtualMachine, e: ssl::Error) -> PyBaseExceptionRef {
    match e.into_io_error() {
        Ok(io_err) => super::os::convert_io_error(vm, io_err),
        Err(e) => convert_openssl_error(vm, e.ssl_error().unwrap().clone()),
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    openssl::init();
    let ctx = &vm.ctx;
    let ssl_error = create_type(
        "SSLError",
        &vm.ctx.types.type_type,
        &vm.ctx.exceptions.os_error,
    );
    py_module!(vm, "_ssl", {
        "_SSLContext" => PySslContext::make_class(ctx),
        "_SSLSocket" => PySslSocket::make_class(ctx),
        "SSLError" => ssl_error,
        "txt2obj" => ctx.new_function(ssl_txt2obj),
        "nid2obj" => ctx.new_function(ssl_nid2obj),
        "get_default_verify_paths" => ctx.new_function(ssl_get_default_verify_paths),

        // Constants
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
    })
}
