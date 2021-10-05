use super::socket::{self, PySocket};
use crate::common::{
    ascii,
    lock::{PyRwLock, PyRwLockWriteGuard},
};
use crate::vm::{
    builtins::{PyBaseException, PyBaseExceptionRef, PyStrRef, PyType, PyTypeRef, PyWeak},
    exceptions::{self, IntoPyException},
    extend_module,
    function::{ArgBytesLike, ArgCallable, ArgMemoryBuffer, ArgStrOrBytesLike, OptionalArg},
    named_function, py_module,
    slots::SlotConstructor,
    stdlib::os::PyPathLike,
    types::create_simple_type,
    utils::{Either, ToCString},
    IntoPyObject, ItemProtocol, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use foreign_types_shared::{ForeignType, ForeignTypeRef};
use openssl::{
    asn1::{Asn1Object, Asn1ObjectRef},
    error::ErrorStack,
    nid::Nid,
    ssl::{self, SslContextBuilder, SslOptions, SslVerifyMode},
    x509::{self, X509Ref, X509},
};
use std::convert::TryFrom;
use std::ffi::CStr;
use std::fmt;
use std::io::{Read, Write};
use std::time::Instant;

use openssl_sys as sys;

mod bio {
    //! based off rust-openssl's private `bio` module

    use super::*;

    use libc::c_int;
    use std::marker::PhantomData;

    pub struct MemBioSlice<'a>(*mut sys::BIO, PhantomData<&'a [u8]>);

    impl<'a> Drop for MemBioSlice<'a> {
        fn drop(&mut self) {
            unsafe {
                sys::BIO_free_all(self.0);
            }
        }
    }

    impl<'a> MemBioSlice<'a> {
        pub fn new(buf: &'a [u8]) -> Result<MemBioSlice<'a>, ErrorStack> {
            openssl::init();

            assert!(buf.len() <= c_int::max_value() as usize);
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
    let no_name = if no_name { 1 } else { 0 };
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
    let s =
        String::from_utf8(b).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    Some(s)
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
    use crate::vm::builtins::PyFrozenSet;
    use schannel::{cert_context::ValidUses, cert_store::CertStore, RawPointer};
    use winapi::um::wincrypt;

    // TODO: check every store for it, not just 2 of them:
    // https://github.com/python/cpython/blob/3.8/Modules/_ssl.c#L5603-L5610
    let open_fns = [CertStore::open_current_user, CertStore::open_local_machine];
    let stores = open_fns
        .iter()
        .filter_map(|open| open(store_name.as_str()).ok())
        .collect::<Vec<_>>();
    let certs = stores.iter().map(|s| s.certs()).flatten().map(|c| {
        let cert = vm.ctx.new_bytes(c.to_der().to_owned());
        let enc_type = unsafe {
            let ptr = c.as_ptr() as wincrypt::PCCERT_CONTEXT;
            (*ptr).dwCertEncodingType
        };
        let enc_type = match enc_type {
            wincrypt::X509_ASN_ENCODING => vm.ctx.new_ascii_literal(ascii!("x509_asn")),
            wincrypt::PKCS_7_ASN_ENCODING => vm.ctx.new_ascii_literal(ascii!("pkcs_7_asn")),
            other => vm.ctx.new_int(other),
        };
        let usage = match c.valid_uses()? {
            ValidUses::All => vm.ctx.new_bool(true),
            ValidUses::Oids(oids) => {
                PyFrozenSet::from_iter(vm, oids.into_iter().map(|oid| vm.ctx.new_utf8_str(oid)))
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
    txt: PyStrRef,
    #[pyarg(any, default = "false")]
    name: bool,
}
fn _ssl_txt2obj(args: Txt2ObjArgs, vm: &VirtualMachine) -> PyResult<PyNid> {
    txt2obj(&args.txt.to_cstring(vm)?, !args.name)
        .as_deref()
        .map(obj2py)
        .ok_or_else(|| vm.new_value_error(format!("unknown object '{}'", args.txt)))
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

fn _ssl_rand_add(string: ArgStrOrBytesLike, entropy: f64) {
    let f = |b: &[u8]| {
        for buf in b.chunks(libc::c_int::max_value() as usize) {
            unsafe { sys::RAND_add(buf.as_ptr() as *const _, buf.len() as _, entropy) }
        }
    };
    f(&string.borrow_bytes())
}

fn _ssl_rand_bytes(n: i32, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    if n < 0 {
        return Err(vm.new_value_error("num must be positive".to_owned()));
    }
    let mut buf = vec![0; n as usize];
    openssl::rand::rand_bytes(&mut buf).map_err(|e| convert_openssl_error(vm, e))?;
    Ok(buf)
}

fn _ssl_rand_pseudo_bytes(n: i32, vm: &VirtualMachine) -> PyResult<(Vec<u8>, bool)> {
    if n < 0 {
        return Err(vm.new_value_error("num must be positive".to_owned()));
    }
    let mut buf = vec![0; n as usize];
    let ret = unsafe { sys::RAND_bytes(buf.as_mut_ptr(), n) };
    match ret {
        0 | 1 => Ok((buf, ret == 1)),
        _ => Err(convert_openssl_error(vm, ErrorStack::get())),
    }
}

#[pyclass(module = "ssl", name = "_SSLContext")]
#[derive(PyValue)]
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

fn builder_as_ctx(x: &SslContextBuilder) -> &ssl::SslContextRef {
    unsafe { ssl::SslContextRef::from_ptr(x.as_ptr()) }
}

impl SlotConstructor for PySslContext {
    type Args = i32;

    fn py_new(cls: PyTypeRef, proto_version: Self::Args, vm: &VirtualMachine) -> PyResult {
        let proto = SslVersion::try_from(proto_version)
            .map_err(|_| vm.new_value_error("invalid protocol version".to_owned()))?;
        let method = match proto {
            // SslVersion::Ssl3 => unsafe { ssl::SslMethod::from_ptr(sys::SSLv3_method()) },
            SslVersion::Tls => ssl::SslMethod::tls(),
            // TODO: Tls1_1, Tls1_2 ?
            SslVersion::TlsClient => ssl::SslMethod::tls_client(),
            SslVersion::TlsServer => ssl::SslMethod::tls_server(),
            _ => return Err(vm.new_value_error("invalid protocol version".to_owned())),
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
        }
        .into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(flags(BASETYPE), with(SlotConstructor))]
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
            let certs = match cadata {
                Either::A(s) => {
                    if !s.is_ascii() {
                        return Err(vm.new_type_error("Must be an ascii string".to_owned()));
                    }
                    X509::stack_from_pem(s.as_str().as_bytes())
                }
                Either::B(b) => b.with_ref(x509_stack_from_der),
            };
            let certs = certs.map_err(|e| convert_openssl_error(vm, e))?;
            let mut ctx = self.builder();
            let store = ctx.cert_store_mut();
            for cert in certs {
                store
                    .add_cert(cert)
                    .map_err(|e| convert_openssl_error(vm, e))?;
            }
        }

        if args.cafile.is_some() || args.capath.is_some() {
            let cafile = args.cafile.map(|s| s.to_cstring(vm)).transpose()?;
            let capath = args.capath.map(|s| s.to_cstring(vm)).transpose()?;
            let ret = unsafe {
                let ctx = self.ctx.write();
                sys::SSL_CTX_load_verify_locations(
                    ctx.as_ptr(),
                    cafile
                        .as_ref()
                        .map_or_else(std::ptr::null, |cs| cs.as_ptr()),
                    capath
                        .as_ref()
                        .map_or_else(std::ptr::null, |cs| cs.as_ptr()),
                )
            };
            if ret != 1 {
                let errno = std::io::Error::last_os_error().raw_os_error().unwrap();
                let err = if errno != 0 {
                    crate::vm::stdlib::os::errno_err(vm)
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
        let binary_form = binary_form.unwrap_or(false);
        self.exec_ctx(|ctx| {
            let certs = ctx
                .cert_store()
                .objects()
                .iter()
                .filter_map(|obj| obj.x509())
                .map(|cert| cert_to_py(vm, cert, binary_form))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(vm.ctx.new_list(certs))
        })
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
            let hostname = hostname.as_str();
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

        let stream = ssl::SslStream::new(ssl, SocketStream(args.sock.clone()))
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
    certfile: PyPathLike,
    #[pyarg(any, optional)]
    keyfile: Option<PyPathLike>,
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

#[pyclass(module = "ssl", name = "_SSLSocket")]
#[derive(PyValue)]
struct PySslSocket {
    ctx: PyRef<PySslContext>,
    stream: PyRwLock<ssl::SslStream<SocketStream>>,
    socket_type: SslServerOrClient,
    server_hostname: Option<PyStrRef>,
    owner: PyRwLock<Option<PyWeak>>,
}

impl fmt::Debug for PySslSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("_SSLSocket")
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
                    ))
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
                ))
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
                    ))
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
                    ))
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
            Either::A(_buf) => vm.ctx.new_int(count),
            Either::B(mut buf) => {
                buf.truncate(n);
                buf.shrink_to_fit();
                vm.ctx.new_bytes(buf)
            }
        };
        Ok(ret)
    }
}

fn ssl_error(vm: &VirtualMachine) -> PyTypeRef {
    vm.class("_ssl", "SSLError")
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
            // TODO: map the error codes to code names, e.g. "CERTIFICATE_VERIFY_FAILED", just requires a big hashmap/dict
            let errstr = e.reason().unwrap_or("unknown error");
            let msg = if let Some(lib) = e.library() {
                format!("[{}] {} ({}:{})", lib, errstr, file, line)
            } else {
                format!("{} ({}:{})", errstr, file, line)
            };
            let reason = sys::ERR_GET_REASON(e.code());
            vm.new_exception(cls, vec![vm.ctx.new_int(reason), vm.ctx.new_utf8_str(msg)])
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

fn x509_stack_from_der(der: &[u8]) -> Result<Vec<X509>, ErrorStack> {
    unsafe {
        openssl::init();
        let bio = bio::MemBioSlice::new(der)?;

        let mut certs = vec![];
        loop {
            let r = sys::d2i_X509_bio(bio.as_ptr(), std::ptr::null_mut());
            if r.is_null() {
                let err = sys::ERR_peek_last_error();
                if sys::ERR_GET_LIB(err) == sys::ERR_LIB_ASN1
                    && sys::ERR_GET_REASON(err) == sys::ASN1_R_HEADER_TOO_LONG
                {
                    sys::ERR_clear_error();
                    break;
                }

                return Err(ErrorStack::get());
            } else {
                certs.push(X509::from_ptr(r));
            }
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
        vm.ctx.new_bytes(b)
    } else {
        let dict = vm.ctx.new_dict();

        let name_to_py = |name: &x509::X509NameRef| {
            name.entries()
                .map(|entry| {
                    let txt = obj2txt(entry.object(), false).into_pyobject(vm);
                    let data = vm.ctx.new_utf8_str(entry.data().as_utf8()?.to_owned());
                    Ok(vm.ctx.new_tuple(vec![vm.ctx.new_tuple(vec![txt, data])]))
                })
                .collect::<Result<_, _>>()
                .map(|list| vm.ctx.new_tuple(list))
                .map_err(|e| convert_openssl_error(vm, e))
        };

        dict.set_item("subject", name_to_py(cert.subject_name())?, vm)?;
        dict.set_item("issuer", name_to_py(cert.issuer_name())?, vm)?;
        dict.set_item("version", vm.ctx.new_int(cert.version()), vm)?;

        let serial_num = cert
            .serial_number()
            .to_bn()
            .and_then(|bn| bn.to_hex_str())
            .map_err(|e| convert_openssl_error(vm, e))?;
        dict.set_item(
            "serialNumber",
            vm.ctx.new_utf8_str(serial_num.to_owned()),
            vm,
        )?;

        dict.set_item(
            "notBefore",
            vm.ctx.new_utf8_str(cert.not_before().to_string()),
            vm,
        )?;
        dict.set_item(
            "notAfter",
            vm.ctx.new_utf8_str(cert.not_after().to_string()),
            vm,
        )?;

        #[allow(clippy::manual_map)]
        if let Some(names) = cert.subject_alt_names() {
            let san = names
                .iter()
                .filter_map(|gen_name| {
                    if let Some(email) = gen_name.email() {
                        Some(vm.ctx.new_tuple(vec![
                            vm.ctx.new_ascii_literal(ascii!("email")),
                            vm.ctx.new_utf8_str(email),
                        ]))
                    } else if let Some(dnsname) = gen_name.dnsname() {
                        Some(vm.ctx.new_tuple(vec![
                            vm.ctx.new_ascii_literal(ascii!("DNS")),
                            vm.ctx.new_utf8_str(dnsname),
                        ]))
                    } else if let Some(ip) = gen_name.ipaddress() {
                        Some(vm.ctx.new_tuple(vec![
                            vm.ctx.new_ascii_literal(ascii!("IP Address")),
                            vm.ctx.new_utf8_str(String::from_utf8_lossy(ip).into_owned()),
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

        dict.into_object()
    };
    Ok(r)
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
    #[cfg(feature = "ssl-vendor")]
    if let None | Some("0") = option_env!("OPENSSL_NO_VENDOR") {
        openssl_probe::init_ssl_cert_env_vars();
    }
    openssl::init();

    // the openssl version from the API headers
    let openssl_api_version = i64::from_str_radix(env!("OPENSSL_API_VERSION"), 16).unwrap();

    let ctx = &vm.ctx;

    let ssl_error = create_simple_type("SSLError", &vm.ctx.exceptions.os_error);

    let ssl_cert_verification_error = PyType::new(
        ctx.types.type_type.clone(),
        "SSLCertVerificationError",
        ssl_error.clone(),
        vec![ssl_error.clone(), ctx.exceptions.value_error.clone()],
        Default::default(),
        PyBaseException::make_slots(),
    )
    .unwrap();
    let ssl_zero_return_error = create_simple_type("SSLZeroReturnError", &ssl_error);
    let ssl_want_read_error = create_simple_type("SSLWantReadError", &ssl_error);
    let ssl_want_write_error = create_simple_type("SSLWantWriteError", &ssl_error);
    let ssl_syscall_error = create_simple_type("SSLSyscallError", &ssl_error);
    let ssl_eof_error = create_simple_type("SSLEOFError", &ssl_error);

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
        "OPENSSL_VERSION" => ctx.new_utf8_str(openssl::version::version()),
        "OPENSSL_VERSION_NUMBER" => ctx.new_int(openssl::version::number()),
        "OPENSSL_VERSION_INFO" => parse_version_info(openssl::version::number()).into_pyobject(vm),
        "_OPENSSL_API_VERSION" => parse_version_info(openssl_api_version).into_pyobject(vm),
        "_DEFAULT_CIPHERS" => ctx.new_utf8_str(DEFAULT_CIPHER_STRING),
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

    #[cfg(ossl111)]
    extend_module!(vm, module, {
        "OP_NO_TLSv1_3" => ctx.new_int(sys::SSL_OP_NO_TLSv1_3),
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
    use crate::{
        exceptions::PyBaseExceptionRef, stdlib::ssl::convert_openssl_error, VirtualMachine,
    };
    use openssl::{
        ssl::SslContextBuilder,
        x509::{store::X509StoreBuilder, X509},
    };
    use std::{
        fs::{read_dir, File},
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
                vm.ctx.exceptions.file_not_found_error.clone(),
                CERT_DIR.to_string(),
            ));
        }

        let mut combined_pem = String::new();
        let entries =
            read_dir(root).map_err(|err| vm.new_os_error(format!("read cert root: {}", err)))?;
        for entry in entries {
            let entry = entry.map_err(|err| vm.new_os_error(format!("iter cert root: {}", err)))?;

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

        let mut store_b = X509StoreBuilder::new().map_err(|err| convert_openssl_error(vm, err))?;
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
