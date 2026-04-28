#[cfg(unix)]
use core::ffi::CStr;
#[cfg(unix)]
use core::time::Duration;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::{io, os::fd::BorrowedFd};

#[cfg(unix)]
#[derive(Copy, Clone)]
pub enum PollKind {
    Read,
    Write,
    Connect,
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn sethostname(hostname: &str) -> io::Result<()> {
    nix::unistd::sethostname(hostname).map_err(io::Error::from)
}

#[cfg(unix)]
pub fn close_socket_ignore_connreset(socket: libc::c_int) -> io::Result<()> {
    let ret = unsafe { libc::close(socket) };
    if ret < 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ECONNRESET) {
            return Err(err);
        }
    }
    Ok(())
}

#[cfg(unix)]
pub fn getsockopt_int(fd: libc::c_int, level: i32, name: i32) -> io::Result<i32> {
    let mut flag: libc::c_int = 0;
    let mut flagsize = core::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(
            fd,
            level,
            name,
            &mut flag as *mut libc::c_int as *mut _,
            &mut flagsize,
        )
    };
    if ret < 0 {
        Err(crate::os::errno_io_error())
    } else {
        Ok(flag)
    }
}

#[cfg(unix)]
pub fn getsockopt_bytes(
    fd: libc::c_int,
    level: i32,
    name: i32,
    buflen: usize,
) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; buflen];
    let mut optlen = buflen as libc::socklen_t;
    let ret = unsafe { libc::getsockopt(fd, level, name, buf.as_mut_ptr() as *mut _, &mut optlen) };
    if ret < 0 {
        Err(crate::os::errno_io_error())
    } else {
        buf.truncate(optlen as usize);
        Ok(buf)
    }
}

#[cfg(unix)]
pub fn setsockopt_bytes(fd: libc::c_int, level: i32, name: i32, value: &[u8]) -> io::Result<()> {
    let ret = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            value.as_ptr() as *const _,
            value.len() as libc::socklen_t,
        )
    };
    if ret < 0 {
        Err(crate::os::errno_io_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub fn setsockopt_int(fd: libc::c_int, level: i32, name: i32, value: i32) -> io::Result<()> {
    let ret = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            &value as *const i32 as *const _,
            core::mem::size_of::<i32>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        Err(crate::os::errno_io_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub fn setsockopt_none(fd: libc::c_int, level: i32, name: i32, optlen: u32) -> io::Result<()> {
    let ret = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            core::ptr::null(),
            optlen as libc::socklen_t,
        )
    };
    if ret < 0 {
        Err(crate::os::errno_io_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub fn poll_socket(
    fd: BorrowedFd<'_>,
    kind: PollKind,
    interval: Option<Duration>,
) -> io::Result<bool> {
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

    let events = match kind {
        PollKind::Read => PollFlags::POLLIN,
        PollKind::Write => PollFlags::POLLOUT,
        PollKind::Connect => PollFlags::POLLOUT | PollFlags::POLLERR,
    };
    let mut pollfd = [PollFd::new(fd, events)];
    let timeout = match interval {
        Some(d) => d.try_into().unwrap_or(PollTimeout::MAX),
        None => PollTimeout::NONE,
    };
    poll(&mut pollfd, timeout)
        .map(|ret| ret == 0)
        .map_err(io::Error::from)
}

#[cfg(any(
    unix,
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "fuchsia",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "netbsd",
    target_os = "openbsd",
))]
pub fn if_nameindex() -> io::Result<Vec<(u32, String)>> {
    let list = nix::net::if_::if_nameindex().map_err(io::Error::from)?;
    Ok(list
        .to_slice()
        .iter()
        .map(|iface| (iface.index(), iface.name().to_string_lossy().into_owned()))
        .collect())
}

#[cfg(unix)]
pub fn if_nametoindex_checked(name: &CStr) -> io::Result<u32> {
    let ret = unsafe { libc::if_nametoindex(name.as_ptr()) };
    if ret == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

#[cfg(unix)]
pub fn if_indextoname_checked(index: u32) -> io::Result<String> {
    let mut buf = [0u8; libc::IF_NAMESIZE];
    let ret = unsafe {
        libc::if_indextoname(index as libc::c_uint, buf.as_mut_ptr() as *mut libc::c_char)
    };
    if ret.is_null() {
        Err(io::Error::last_os_error())
    } else {
        let buf = unsafe { CStr::from_ptr(buf.as_ptr() as *const libc::c_char) };
        Ok(buf.to_string_lossy().into_owned())
    }
}

#[cfg(unix)]
pub fn gai_error_string(err: i32) -> String {
    unsafe { CStr::from_ptr(libc::gai_strerror(err)) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(unix)]
pub fn h_error_string(err: i32) -> String {
    unsafe { CStr::from_ptr(libc::hstrerror(err)) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(all(unix, not(target_os = "redox")))]
#[derive(Debug, Clone)]
pub struct AncillaryMessage {
    pub level: i32,
    pub kind: i32,
    pub data: Vec<u8>,
}

#[cfg(all(unix, not(target_os = "redox")))]
pub type SocketAddressBytes = [u8; core::mem::size_of::<libc::sockaddr_storage>()];

#[cfg(all(unix, not(target_os = "redox")))]
#[derive(Debug, Clone)]
pub struct RawSocketAddress {
    pub storage: SocketAddressBytes,
    pub len: usize,
}

#[cfg(all(unix, not(target_os = "redox")))]
#[derive(Debug, Clone)]
pub struct RecvMsgResult {
    pub data: Vec<u8>,
    pub ancdata: Vec<AncillaryMessage>,
    pub msg_flags: i32,
    pub address: Option<RawSocketAddress>,
}

#[cfg(all(unix, not(target_os = "redox")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AncillaryPackError {
    ItemTooLarge,
    TooMuchData,
    UnexpectedNullHeader,
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn checked_cmsg_len(len: usize) -> Option<usize> {
    let cmsg_len = |length| unsafe { libc::CMSG_LEN(length) };
    if len as u64 > (i32::MAX as u64 - cmsg_len(0) as u64) {
        return None;
    }
    let res = cmsg_len(len as _) as usize;
    if res > i32::MAX as usize || res < len {
        return None;
    }
    Some(res)
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn checked_cmsg_space(len: usize) -> Option<usize> {
    let cmsg_space = |length| unsafe { libc::CMSG_SPACE(length) };
    if len as u64 > (i32::MAX as u64 - cmsg_space(1) as u64) {
        return None;
    }
    let res = cmsg_space(len as _) as usize;
    if res > i32::MAX as usize || res < len {
        return None;
    }
    Some(res)
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn pack_ancillary_messages(cmsgs: &[(i32, i32, &[u8])]) -> Result<Vec<u8>, AncillaryPackError> {
    use core::{mem, ptr};

    if cmsgs.is_empty() {
        return Ok(vec![]);
    }

    let capacity = cmsgs
        .iter()
        .map(|(_, _, buf)| buf.len())
        .try_fold(0usize, |sum, len| {
            let space = checked_cmsg_space(len).ok_or(AncillaryPackError::ItemTooLarge)?;
            usize::checked_add(sum, space).ok_or(AncillaryPackError::TooMuchData)
        })?;

    let mut cmsg_buffer = vec![0u8; capacity];
    let mut mhdr = unsafe { mem::zeroed::<libc::msghdr>() };
    mhdr.msg_control = cmsg_buffer.as_mut_ptr().cast();
    mhdr.msg_controllen = capacity as _;

    let mut pmhdr: *mut libc::cmsghdr = unsafe { libc::CMSG_FIRSTHDR(&mhdr) };
    for (lvl, typ, data) in cmsgs {
        if pmhdr.is_null() {
            return Err(AncillaryPackError::UnexpectedNullHeader);
        }
        let cmsg_len = checked_cmsg_len(data.len()).ok_or(AncillaryPackError::ItemTooLarge)?;
        unsafe {
            (*pmhdr).cmsg_level = *lvl;
            (*pmhdr).cmsg_type = *typ;
            (*pmhdr).cmsg_len = cmsg_len as _;
            ptr::copy_nonoverlapping(data.as_ptr(), libc::CMSG_DATA(pmhdr), data.len());
            pmhdr = libc::CMSG_NXTHDR(&mhdr, pmhdr);
        }
    }

    Ok(cmsg_buffer)
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn parse_ancillary_messages(control: &[u8]) -> Vec<AncillaryMessage> {
    use core::mem;

    if control.is_empty() {
        return Vec::new();
    }

    let mut msg = unsafe { mem::zeroed::<libc::msghdr>() };
    msg.msg_control = control.as_ptr() as *mut _;
    msg.msg_controllen = control.len() as _;

    let ctrl_buf = msg.msg_control as *const u8;
    let ctrl_end = unsafe { ctrl_buf.add(msg.msg_controllen as _) };

    let mut result = Vec::new();
    let mut cmsg: *mut libc::cmsghdr = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    while !cmsg.is_null() {
        let cmsg_ref = unsafe { &*cmsg };
        let data_ptr = unsafe { libc::CMSG_DATA(cmsg) };
        let data_len_from_cmsg = cmsg_ref.cmsg_len as usize - (data_ptr as usize - cmsg as usize);
        let available = ctrl_end as usize - data_ptr as usize;
        let data_len = data_len_from_cmsg.min(available);
        let data = unsafe { core::slice::from_raw_parts(data_ptr, data_len) };
        result.push(AncillaryMessage {
            level: cmsg_ref.cmsg_level,
            kind: cmsg_ref.cmsg_type,
            data: data.to_vec(),
        });
        cmsg = unsafe { libc::CMSG_NXTHDR(&msg, cmsg) };
    }

    result
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn recvmsg(
    fd: BorrowedFd<'_>,
    bufsize: usize,
    ancbufsize: usize,
    flags: i32,
) -> io::Result<RecvMsgResult> {
    use core::mem::MaybeUninit;

    let mut data_buf: Vec<MaybeUninit<u8>> = vec![MaybeUninit::uninit(); bufsize];
    let mut anc_buf: Vec<MaybeUninit<u8>> = vec![MaybeUninit::uninit(); ancbufsize];
    let mut addr_storage: libc::sockaddr_storage = unsafe { core::mem::zeroed() };

    let mut iov = [libc::iovec {
        iov_base: data_buf.as_mut_ptr().cast(),
        iov_len: bufsize,
    }];

    let mut msg: libc::msghdr = unsafe { core::mem::zeroed() };
    msg.msg_name = (&mut addr_storage as *mut libc::sockaddr_storage).cast();
    msg.msg_namelen = core::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    msg.msg_iov = iov.as_mut_ptr();
    msg.msg_iovlen = 1;
    if ancbufsize > 0 {
        msg.msg_control = anc_buf.as_mut_ptr().cast();
        msg.msg_controllen = ancbufsize as _;
    }

    let ret = unsafe { libc::recvmsg(fd.as_raw_fd(), &mut msg, flags) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }

    let data = unsafe {
        data_buf.set_len(ret as usize);
        core::mem::transmute::<Vec<MaybeUninit<u8>>, Vec<u8>>(data_buf)
    };
    let control = unsafe {
        core::slice::from_raw_parts(anc_buf.as_ptr().cast::<u8>(), msg.msg_controllen as usize)
    };
    let ancdata = parse_ancillary_messages(control);
    let address = if msg.msg_namelen > 0 {
        let storage = unsafe {
            core::mem::transmute::<libc::sockaddr_storage, SocketAddressBytes>(addr_storage)
        };
        Some(RawSocketAddress {
            storage,
            len: msg.msg_namelen as usize,
        })
    } else {
        None
    };

    Ok(RecvMsgResult {
        data,
        ancdata,
        msg_flags: msg.msg_flags,
        address,
    })
}

#[cfg(target_os = "linux")]
pub fn sendmsg_afalg(
    fd: BorrowedFd<'_>,
    buffers: &[io::IoSlice<'_>],
    op: u32,
    iv: Option<&[u8]>,
    assoclen: Option<u32>,
    flags: i32,
) -> io::Result<usize> {
    let mut control_buf = Vec::new();

    {
        let op_bytes = op.to_ne_bytes();
        let space = unsafe { libc::CMSG_SPACE(core::mem::size_of::<u32>() as u32) } as usize;
        let old_len = control_buf.len();
        control_buf.resize(old_len + space, 0u8);

        let cmsg = control_buf[old_len..].as_mut_ptr() as *mut libc::cmsghdr;
        unsafe {
            (*cmsg).cmsg_len = libc::CMSG_LEN(core::mem::size_of::<u32>() as u32) as _;
            (*cmsg).cmsg_level = libc::SOL_ALG;
            (*cmsg).cmsg_type = libc::ALG_SET_OP;
            let data = libc::CMSG_DATA(cmsg);
            core::ptr::copy_nonoverlapping(op_bytes.as_ptr(), data, op_bytes.len());
        }
    }

    if let Some(iv_bytes) = iv {
        let iv_struct_size = 4 + iv_bytes.len();
        let space = unsafe { libc::CMSG_SPACE(iv_struct_size as u32) } as usize;
        let old_len = control_buf.len();
        control_buf.resize(old_len + space, 0u8);

        let cmsg = control_buf[old_len..].as_mut_ptr() as *mut libc::cmsghdr;
        unsafe {
            (*cmsg).cmsg_len = libc::CMSG_LEN(iv_struct_size as u32) as _;
            (*cmsg).cmsg_level = libc::SOL_ALG;
            (*cmsg).cmsg_type = libc::ALG_SET_IV;
            let data = libc::CMSG_DATA(cmsg);
            let ivlen = (iv_bytes.len() as u32).to_ne_bytes();
            core::ptr::copy_nonoverlapping(ivlen.as_ptr(), data, 4);
            core::ptr::copy_nonoverlapping(iv_bytes.as_ptr(), data.add(4), iv_bytes.len());
        }
    }

    if let Some(assoclen_val) = assoclen {
        let assoclen_bytes = assoclen_val.to_ne_bytes();
        let space = unsafe { libc::CMSG_SPACE(core::mem::size_of::<u32>() as u32) } as usize;
        let old_len = control_buf.len();
        control_buf.resize(old_len + space, 0u8);

        let cmsg = control_buf[old_len..].as_mut_ptr() as *mut libc::cmsghdr;
        unsafe {
            (*cmsg).cmsg_len = libc::CMSG_LEN(core::mem::size_of::<u32>() as u32) as _;
            (*cmsg).cmsg_level = libc::SOL_ALG;
            (*cmsg).cmsg_type = libc::ALG_SET_AEAD_ASSOCLEN;
            let data = libc::CMSG_DATA(cmsg);
            core::ptr::copy_nonoverlapping(assoclen_bytes.as_ptr(), data, assoclen_bytes.len());
        }
    }

    let iovecs: Vec<libc::iovec> = buffers
        .iter()
        .map(|buf| libc::iovec {
            iov_base: buf.as_ptr() as *mut _,
            iov_len: buf.len(),
        })
        .collect();

    let mut msghdr: libc::msghdr = unsafe { core::mem::zeroed() };
    msghdr.msg_iov = iovecs.as_ptr() as *mut _;
    msghdr.msg_iovlen = iovecs.len() as _;
    if !control_buf.is_empty() {
        msghdr.msg_control = control_buf.as_mut_ptr() as *mut _;
        msghdr.msg_controllen = control_buf.len() as _;
    }

    let ret = unsafe { libc::sendmsg(fd.as_raw_fd(), &msghdr, flags) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret as usize)
    }
}

#[cfg(windows)]
use core::{ffi::CStr, ptr::NonNull};
#[cfg(windows)]
use std::io;
#[cfg(windows)]
use windows_sys::Win32::{
    NetworkManagement::{
        IpHelper::{
            ConvertInterfaceLuidToNameW, FreeMibTable, GetIfTable2Ex, MIB_IF_ROW2, MIB_IF_TABLE2,
            MibIfTableRaw, if_indextoname, if_nametoindex,
        },
        Ndis::{IF_MAX_STRING_SIZE, NET_LUID_LH},
    },
    Networking::WinSock::{
        FROM_PROTOCOL_INFO, INVALID_SOCKET, SOCKET, SOCKET_ERROR, WSA_FLAG_OVERLAPPED,
        WSADuplicateSocketW, WSAGetLastError, WSAIoctl, WSAPROTOCOL_INFOW, WSASocketW,
    },
};

#[cfg(windows)]
pub use windows_sys::Win32::Networking::WinSock::{
    AF_APPLETALK, AF_DECnet, AF_IPX, AF_LINK, AI_ADDRCONFIG, AI_ALL, AI_CANONNAME, AI_NUMERICSERV,
    AI_V4MAPPED, INADDR_ANY, INADDR_BROADCAST, INADDR_LOOPBACK, INADDR_NONE, IPPORT_RESERVED,
    IPPROTO_AH, IPPROTO_CBT, IPPROTO_DSTOPTS, IPPROTO_EGP, IPPROTO_ESP, IPPROTO_FRAGMENT,
    IPPROTO_GGP, IPPROTO_HOPOPTS, IPPROTO_ICLFXBM, IPPROTO_ICMP, IPPROTO_ICMPV6, IPPROTO_IDP,
    IPPROTO_IGMP, IPPROTO_IGP, IPPROTO_IP, IPPROTO_IP as IPPROTO_IPIP, IPPROTO_IPV4, IPPROTO_IPV6,
    IPPROTO_L2TP, IPPROTO_ND, IPPROTO_NONE, IPPROTO_PGM, IPPROTO_PIM, IPPROTO_PUP, IPPROTO_RAW,
    IPPROTO_RDP, IPPROTO_ROUTING, IPPROTO_SCTP, IPPROTO_ST, IPPROTO_TCP, IPPROTO_UDP,
    IPV6_CHECKSUM, IPV6_DONTFRAG, IPV6_HOPLIMIT, IPV6_HOPOPTS, IPV6_JOIN_GROUP, IPV6_LEAVE_GROUP,
    IPV6_MULTICAST_HOPS, IPV6_MULTICAST_IF, IPV6_MULTICAST_LOOP, IPV6_PKTINFO, IPV6_RECVRTHDR,
    IPV6_RECVTCLASS, IPV6_RTHDR, IPV6_TCLASS, IPV6_UNICAST_HOPS, IPV6_V6ONLY, MSG_BCAST,
    MSG_CTRUNC, MSG_DONTROUTE, MSG_MCAST, MSG_OOB, MSG_PEEK, MSG_TRUNC, MSG_WAITALL, NI_DGRAM,
    NI_MAXHOST, NI_MAXSERV, NI_NAMEREQD, NI_NOFQDN, NI_NUMERICHOST, NI_NUMERICSERV, RCVALL_IPLEVEL,
    RCVALL_OFF, RCVALL_ON, RCVALL_SOCKETLEVELONLY, SD_BOTH, SD_RECEIVE, SD_SEND,
    SIO_KEEPALIVE_VALS, SIO_LOOPBACK_FAST_PATH, SIO_RCVALL, SO_BROADCAST, SO_ERROR, SO_KEEPALIVE,
    SO_LINGER, SO_OOBINLINE, SO_RCVBUF, SO_REUSEADDR, SO_SNDBUF, SO_TYPE, SO_USELOOPBACK,
    SOCK_DGRAM, SOCK_RAW, SOCK_RDM, SOCK_SEQPACKET, SOCK_STREAM, SOCKET_ERROR as SOCKET_ERROR_CODE,
    SOL_SOCKET, SOMAXCONN, TCP_NODELAY, WSAEBADF, WSAECONNRESET, WSAENOTSOCK, WSAEWOULDBLOCK,
    getprotobyname, getservbyname, getservbyport, getsockopt, setsockopt,
};

#[cfg(windows)]
pub const SO_EXCLUSIVEADDRUSE: i32 = SO_REUSEADDR;
#[cfg(windows)]
pub const EAI_MEMORY: i32 = windows_sys::Win32::Networking::WinSock::WSA_NOT_ENOUGH_MEMORY;
#[cfg(windows)]
pub const EAI_FAMILY: i32 = windows_sys::Win32::Networking::WinSock::WSAEAFNOSUPPORT;
#[cfg(windows)]
pub const EAI_BADFLAGS: i32 = windows_sys::Win32::Networking::WinSock::WSAEINVAL;
#[cfg(windows)]
pub const EAI_SOCKTYPE: i32 = windows_sys::Win32::Networking::WinSock::WSAESOCKTNOSUPPORT;
#[cfg(windows)]
pub const EAI_NODATA: i32 = windows_sys::Win32::Networking::WinSock::WSAHOST_NOT_FOUND;
#[cfg(windows)]
pub const EAI_NONAME: i32 = windows_sys::Win32::Networking::WinSock::WSAHOST_NOT_FOUND;
#[cfg(windows)]
pub const EAI_FAIL: i32 = windows_sys::Win32::Networking::WinSock::WSANO_RECOVERY;
#[cfg(windows)]
pub const EAI_AGAIN: i32 = windows_sys::Win32::Networking::WinSock::WSATRY_AGAIN;
#[cfg(windows)]
pub const EAI_SERVICE: i32 = windows_sys::Win32::Networking::WinSock::WSATYPE_NOT_FOUND;
#[cfg(windows)]
pub const IF_NAMESIZE: usize = IF_MAX_STRING_SIZE as usize;
#[cfg(windows)]
pub const AF_UNSPEC: i32 = windows_sys::Win32::Networking::WinSock::AF_UNSPEC as i32;
#[cfg(windows)]
pub const AF_INET: i32 = windows_sys::Win32::Networking::WinSock::AF_INET as i32;
#[cfg(windows)]
pub const AF_INET6: i32 = windows_sys::Win32::Networking::WinSock::AF_INET6 as i32;
#[cfg(windows)]
pub const AI_PASSIVE: i32 = windows_sys::Win32::Networking::WinSock::AI_PASSIVE as i32;
#[cfg(windows)]
pub const AI_NUMERICHOST: i32 = windows_sys::Win32::Networking::WinSock::AI_NUMERICHOST as i32;
#[cfg(windows)]
pub const FROM_PROTOCOL_INFO_VALUE: i32 = FROM_PROTOCOL_INFO;

#[cfg(windows)]
pub type RawSocket = SOCKET;

#[cfg(windows)]
pub const INVALID_RAW_SOCKET: RawSocket = INVALID_SOCKET as RawSocket;

#[cfg(windows)]
#[repr(C)]
pub struct TcpKeepalive {
    pub onoff: u32,
    pub keepalivetime: u32,
    pub keepaliveinterval: u32,
}

#[cfg(windows)]
pub struct SharedSocket {
    pub raw: RawSocket,
    pub family: i32,
    pub socket_type: i32,
    pub protocol: i32,
}

#[cfg(windows)]
pub fn last_socket_error() -> io::Error {
    io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
}

#[cfg(windows)]
pub fn set_socket_inheritable(socket: RawSocket, inheritable: bool) -> io::Result<()> {
    crate::nt::set_handle_inheritable(socket as _, inheritable)
}

#[cfg(windows)]
pub fn close_socket_ignore_connreset(socket: RawSocket) -> io::Result<()> {
    let ret = unsafe { windows_sys::Win32::Networking::WinSock::closesocket(socket) };
    if ret != 0 {
        let err = last_socket_error();
        if err.raw_os_error() != Some(WSAECONNRESET) {
            return Err(err);
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn getsockopt_int(socket: RawSocket, level: i32, name: i32) -> io::Result<i32> {
    let mut flag = 0i32;
    let mut optlen = core::mem::size_of::<i32>() as i32;
    let ret = unsafe {
        getsockopt(
            socket,
            level,
            name,
            &mut flag as *mut i32 as *mut _,
            &mut optlen,
        )
    };
    if ret == SOCKET_ERROR {
        Err(crate::os::errno_io_error())
    } else {
        Ok(flag)
    }
}

#[cfg(windows)]
pub fn getsockopt_bytes(
    socket: RawSocket,
    level: i32,
    name: i32,
    buflen: usize,
) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; buflen];
    let mut optlen = buflen as i32;
    let ret = unsafe { getsockopt(socket, level, name, buf.as_mut_ptr() as *mut _, &mut optlen) };
    if ret == SOCKET_ERROR {
        Err(crate::os::errno_io_error())
    } else {
        buf.truncate(optlen as usize);
        Ok(buf)
    }
}

#[cfg(windows)]
pub fn setsockopt_bytes(socket: RawSocket, level: i32, name: i32, value: &[u8]) -> io::Result<()> {
    let ret = unsafe {
        setsockopt(
            socket,
            level,
            name,
            value.as_ptr() as *const _,
            value.len() as i32,
        )
    };
    if ret == SOCKET_ERROR {
        Err(crate::os::errno_io_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
pub fn setsockopt_int(socket: RawSocket, level: i32, name: i32, value: i32) -> io::Result<()> {
    let ret = unsafe {
        setsockopt(
            socket,
            level,
            name,
            &value as *const i32 as *const _,
            core::mem::size_of::<i32>() as i32,
        )
    };
    if ret == SOCKET_ERROR {
        Err(crate::os::errno_io_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
pub fn setsockopt_none(socket: RawSocket, level: i32, name: i32, optlen: u32) -> io::Result<()> {
    let ret = unsafe { setsockopt(socket, level, name, core::ptr::null(), optlen as i32) };
    if ret == SOCKET_ERROR {
        Err(crate::os::errno_io_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
pub fn protocol_info_size() -> usize {
    core::mem::size_of::<WSAPROTOCOL_INFOW>()
}

#[cfg(windows)]
pub fn socket_from_share_data(bytes: &[u8]) -> io::Result<SharedSocket> {
    let mut info: WSAPROTOCOL_INFOW = unsafe { core::mem::zeroed() };
    unsafe {
        core::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            &mut info as *mut WSAPROTOCOL_INFOW as *mut u8,
            protocol_info_size(),
        );
    }

    let raw = unsafe {
        WSASocketW(
            FROM_PROTOCOL_INFO,
            FROM_PROTOCOL_INFO,
            FROM_PROTOCOL_INFO,
            &info,
            0,
            WSA_FLAG_OVERLAPPED,
        )
    };
    if raw == INVALID_SOCKET {
        return Err(last_socket_error());
    }

    crate::nt::set_handle_inheritable(raw as _, false)?;

    Ok(SharedSocket {
        raw,
        family: info.iAddressFamily,
        socket_type: info.iSocketType,
        protocol: info.iProtocol,
    })
}

#[cfg(windows)]
pub fn share_socket(socket: RawSocket, process_id: u32) -> io::Result<Vec<u8>> {
    let mut info = core::mem::MaybeUninit::<WSAPROTOCOL_INFOW>::uninit();
    let ret = unsafe { WSADuplicateSocketW(socket, process_id, info.as_mut_ptr()) };
    if ret == SOCKET_ERROR {
        return Err(last_socket_error());
    }
    let info = unsafe { info.assume_init() };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            &info as *const WSAPROTOCOL_INFOW as *const u8,
            core::mem::size_of::<WSAPROTOCOL_INFOW>(),
        )
    };
    Ok(bytes.to_vec())
}

#[cfg(windows)]
pub fn ioctl_u32(socket: RawSocket, cmd: u32, option: u32) -> io::Result<u32> {
    let mut recv = 0u32;
    let ret = unsafe {
        WSAIoctl(
            socket,
            cmd,
            &option as *const u32 as *const _,
            core::mem::size_of::<u32>() as u32,
            core::ptr::null_mut(),
            0,
            &mut recv,
            core::ptr::null_mut(),
            None,
        )
    };
    if ret == SOCKET_ERROR {
        Err(last_socket_error())
    } else {
        Ok(recv)
    }
}

#[cfg(windows)]
pub fn ioctl_keepalive(socket: RawSocket, keepalive: TcpKeepalive) -> io::Result<u32> {
    let mut recv = 0u32;
    let ret = unsafe {
        WSAIoctl(
            socket,
            windows_sys::Win32::Networking::WinSock::SIO_KEEPALIVE_VALS,
            &keepalive as *const TcpKeepalive as *const _,
            core::mem::size_of::<TcpKeepalive>() as u32,
            core::ptr::null_mut(),
            0,
            &mut recv,
            core::ptr::null_mut(),
            None,
        )
    };
    if ret == SOCKET_ERROR {
        Err(last_socket_error())
    } else {
        Ok(recv)
    }
}

#[cfg(windows)]
pub fn if_nametoindex_checked(name: &CStr) -> io::Result<u32> {
    crate::os::set_errno(libc::ENODEV);
    let ret = unsafe { if_nametoindex(name.as_ptr() as _) };
    if ret == 0 {
        Err(crate::os::errno_io_error())
    } else {
        Ok(ret)
    }
}

#[cfg(windows)]
pub fn if_indextoname_checked(index: u32) -> io::Result<String> {
    let mut buf = [0; IF_MAX_STRING_SIZE as usize + 1];
    crate::os::set_errno(libc::ENXIO);
    let ret = unsafe { if_indextoname(index, buf.as_mut_ptr()) };
    if ret.is_null() {
        Err(crate::os::errno_io_error())
    } else {
        let buf = unsafe { CStr::from_ptr(buf.as_ptr() as _) };
        Ok(buf.to_string_lossy().into_owned())
    }
}

#[cfg(windows)]
pub fn if_nameindex() -> io::Result<Vec<(u32, String)>> {
    fn get_name(luid: &NET_LUID_LH) -> io::Result<String> {
        let mut buf = [0u16; IF_MAX_STRING_SIZE as usize + 1];
        let ret = unsafe { ConvertInterfaceLuidToNameW(luid, buf.as_mut_ptr(), buf.len()) };
        if ret != 0 {
            return Err(io::Error::from_raw_os_error(ret as i32));
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(String::from_utf16_lossy(&buf[..len]))
    }

    struct MibTable {
        ptr: NonNull<MIB_IF_TABLE2>,
    }

    impl MibTable {
        fn get_raw() -> io::Result<Self> {
            let mut ptr = core::ptr::null_mut();
            let ret = unsafe { GetIfTable2Ex(MibIfTableRaw, &mut ptr) };
            if ret == 0 {
                let ptr = unsafe { NonNull::new_unchecked(ptr) };
                Ok(Self { ptr })
            } else {
                Err(io::Error::from_raw_os_error(ret as i32))
            }
        }

        fn as_slice(&self) -> &[MIB_IF_ROW2] {
            unsafe {
                let p = self.ptr.as_ptr();
                let ptr = &raw const (*p).Table as *const MIB_IF_ROW2;
                core::slice::from_raw_parts(ptr, (*p).NumEntries as usize)
            }
        }
    }

    impl Drop for MibTable {
        fn drop(&mut self) {
            unsafe { FreeMibTable(self.ptr.as_ptr() as *mut _) };
        }
    }

    let table = MibTable::get_raw()?;
    table
        .as_slice()
        .iter()
        .map(|entry| Ok((entry.InterfaceIndex, get_name(&entry.InterfaceLuid)?)))
        .collect()
}
