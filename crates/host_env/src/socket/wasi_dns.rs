//! WASI p2 resolver adapter for the shared Python socket implementation.
//! Resolver allocations are owned locally; EAI_SYSTEM preserves original errno.
use std::{
    ffi::{CStr, CString},
    io, mem,
    net::SocketAddr,
    ptr,
};

#[derive(Default)]
pub struct AddrInfoHints {
    pub socktype: i32,
    pub protocol: i32,
    pub address: i32,
    pub flags: i32,
}

pub struct AddrInfo {
    pub address: i32,
    pub socktype: i32,
    pub protocol: i32,
    pub sockaddr: SocketAddr,
    pub canonname: Option<String>,
}

#[derive(Debug)]
pub enum LookupErrorKind {
    System,
    Resolver,
}

#[derive(Debug)]
pub struct LookupError {
    code: i32,
    system: Option<io::Error>,
}
impl LookupError {
    pub fn kind(&self) -> LookupErrorKind {
        if self.system.is_some() {
            LookupErrorKind::System
        } else {
            LookupErrorKind::Resolver
        }
    }
    pub fn error_num(&self) -> i32 {
        self.code
    }
    fn from_code(code: i32) -> Self {
        // Snapshot errno before allocation or freeaddrinfo can overwrite it.
        Self {
            code,
            system: (code == libc::EAI_SYSTEM).then(io::Error::last_os_error),
        }
    }
}
impl From<LookupError> for io::Error {
    fn from(error: LookupError) -> Self {
        error
            .system
            .unwrap_or_else(|| io::Error::other(gai_error_string(error.code)))
    }
}
impl From<std::ffi::NulError> for LookupError {
    fn from(error: std::ffi::NulError) -> Self {
        Self {
            code: 0,
            system: Some(io::Error::new(io::ErrorKind::InvalidInput, error)),
        }
    }
}

pub fn gai_error_string(code: i32) -> String {
    // SAFETY: gai_strerror returns a static NUL-terminated string or null.
    let text = unsafe { libc::gai_strerror(code) };
    if text.is_null() {
        format!("getaddrinfo error {code}")
    } else {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    }
}

struct AddrInfoList(*mut libc::addrinfo);
impl Drop for AddrInfoList {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: owns precisely the list returned by successful getaddrinfo.
            unsafe { libc::freeaddrinfo(self.0) };
        }
    }
}

fn copy_address(info: &libc::addrinfo) -> io::Result<SocketAddr> {
    let required = match info.ai_family {
        libc::AF_INET => mem::size_of::<libc::sockaddr_in>(),
        libc::AF_INET6 => mem::size_of::<libc::sockaddr_in6>(),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected address family",
            ));
        }
    };
    let length = info.ai_addrlen as usize;
    let mut storage = socket2::SockAddrStorage::zeroed();
    if info.ai_addr.is_null() || length < required || length > mem::size_of_val(&storage) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid resolver address length",
        ));
    }
    // SAFETY: getaddrinfo owns ai_addrlen initialized address bytes; storage has
    // sufficient size and alignment. The copied address remains independent of
    // the resolver list. SockAddr's family is checked against the length below.
    unsafe {
        ptr::copy_nonoverlapping(
            info.ai_addr.cast::<u8>(),
            ptr::from_mut(&mut storage).cast(),
            length,
        );
    }
    // Check initialized storage before invoking the unsafe SockAddr constructor.
    let stored_family = unsafe { storage.view_as::<libc::sockaddr>() }.sa_family;
    if i32::from(stored_family) != info.ai_family {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "inconsistent resolver address family",
        ));
    }
    let address = unsafe { socket2::SockAddr::new(storage, info.ai_addrlen) };
    address
        .as_socket()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an IP address"))
}

pub fn getaddrinfo(
    host: Option<&str>,
    service: Option<&str>,
    hints: Option<AddrInfoHints>,
) -> Result<std::vec::IntoIter<io::Result<AddrInfo>>, LookupError> {
    // Strict shared adapter contract. Python getaddrinfo alone may truncate
    // its converted inputs at NUL; address-taking/getnameinfo must reject NUL.
    let host = host.map(CString::new).transpose()?;
    let service = service.map(CString::new).transpose()?;
    // SAFETY: zero is valid for every field in the C hints structure.
    let mut raw_hints: libc::addrinfo = unsafe { mem::zeroed() };
    let hints_ptr = if let Some(hints) = hints {
        raw_hints.ai_flags = hints.flags;
        raw_hints.ai_family = hints.address;
        raw_hints.ai_socktype = hints.socktype;
        raw_hints.ai_protocol = hints.protocol;
        ptr::from_ref(&raw_hints)
    } else {
        ptr::null()
    };
    let mut result = ptr::null_mut();
    // SAFETY: input C strings and hints remain live for the call, and result is
    // a writable out pointer. A null option remains null rather than an empty string.
    let code = unsafe {
        libc::getaddrinfo(
            host.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            service.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            hints_ptr,
            &mut result,
        )
    };
    if code != 0 {
        return Err(LookupError::from_code(code));
    }
    let list = AddrInfoList(result);
    let mut current = list.0;
    let mut addresses = Vec::new();
    while !current.is_null() {
        // SAFETY: successful getaddrinfo returns a linked list owned by list.
        let info = unsafe { &*current };
        addresses.push(copy_address(info).map(|sockaddr| AddrInfo {
            address: info.ai_family,
            socktype: info.ai_socktype,
            protocol: info.ai_protocol,
            sockaddr,
            canonname: if info.ai_canonname.is_null() {
                None
            } else {
                Some(
                    unsafe { CStr::from_ptr(info.ai_canonname) }
                        .to_string_lossy()
                        .into_owned(),
                )
            },
        }));
        current = info.ai_next;
    }
    // All fields are copied; libc storage is freed before returning the iterator.
    Ok(addresses.into_iter())
}

// libc 0.2.189 omits this declaration for WASI although wasi-libc netdb.h
// declares it. Keep the ABI declaration in the host adapter, not stdlib.
unsafe extern "C" {
    #[link_name = "getnameinfo"]
    fn c_getnameinfo(
        addr: *const libc::sockaddr,
        len: libc::socklen_t,
        host: *mut libc::c_char,
        hostlen: libc::socklen_t,
        service: *mut libc::c_char,
        servicelen: libc::socklen_t,
        flags: libc::c_int,
    ) -> libc::c_int;
}

pub fn getnameinfo(address: &SocketAddr, flags: i32) -> Result<(String, String), LookupError> {
    let address = socket2::SockAddr::from(*address);
    // NI_MAXHOST and NI_MAXSERV from wasi-libc netdb.h. A too-long result must
    // return EAI_OVERFLOW; never expose uninitialized bytes or a truncated name.
    let mut host = [0_u8; 255];
    let mut service = [0_u8; 32];
    let code = unsafe {
        c_getnameinfo(
            address.as_ptr().cast(),
            address.len(),
            host.as_mut_ptr().cast(),
            host.len() as _,
            service.as_mut_ptr().cast(),
            service.len() as _,
            flags,
        )
    };
    if code != 0 {
        return Err(LookupError::from_code(code));
    }
    let parse = |buffer: &[u8]| {
        CStr::from_bytes_until_nul(buffer)
            .map(|text| text.to_string_lossy().into_owned())
            .map_err(|_| LookupError {
                code: 0,
                system: Some(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unterminated resolver output",
                )),
            })
    };
    Ok((parse(&host)?, parse(&service)?))
}
