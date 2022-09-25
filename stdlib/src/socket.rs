use crate::vm::{PyObjectRef, VirtualMachine};
#[cfg(feature = "ssl")]
pub(super) use _socket::{sock_select, timeout_error_msg, PySocket, SelectKind};

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    #[cfg(windows)]
    crate::vm::stdlib::nt::init_winsock();
    _socket::make_module(vm)
}

#[pymodule]
mod _socket {
    use crate::common::lock::{PyMappedRwLockReadGuard, PyRwLock, PyRwLockReadGuard};
    use crate::vm::{
        builtins::{PyBaseExceptionRef, PyListRef, PyStrRef, PyTupleRef, PyTypeRef},
        convert::{IntoPyException, ToPyObject, TryFromBorrowedObject, TryFromObject},
        function::{ArgBytesLike, ArgMemoryBuffer, Either, OptionalArg, OptionalOption},
        types::{DefaultConstructor, Initializer},
        utils::ToCString,
        AsObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use num_traits::ToPrimitive;
    use socket2::{Domain, Protocol, Socket, Type as SocketType};
    use std::{
        ffi,
        io::{self, Read, Write},
        mem::MaybeUninit,
        net::{self, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, ToSocketAddrs},
        time::{Duration, Instant},
    };

    #[cfg(unix)]
    use libc as c;
    #[cfg(windows)]
    mod c {
        pub use winapi::shared::ifdef::IF_MAX_STRING_SIZE as IF_NAMESIZE;
        pub use winapi::shared::mstcpip::*;
        pub use winapi::shared::netioapi::{if_indextoname, if_nametoindex};
        pub use winapi::shared::ws2def::*;
        pub use winapi::shared::ws2ipdef::*;
        pub use winapi::um::winsock2::{
            IPPORT_RESERVED, SD_BOTH as SHUT_RDWR, SD_RECEIVE as SHUT_RD, SD_SEND as SHUT_WR,
            SOCK_DGRAM, SOCK_RAW, SOCK_RDM, SOCK_SEQPACKET, SOCK_STREAM, SOL_SOCKET, SO_BROADCAST,
            SO_ERROR, SO_EXCLUSIVEADDRUSE, SO_LINGER, SO_OOBINLINE, SO_REUSEADDR, SO_TYPE,
            SO_USELOOPBACK, *,
        };
        pub use winapi::um::ws2tcpip::*;
    }
    // constants
    #[pyattr(name = "has_ipv6")]
    const HAS_IPV6: bool = true;
    #[pyattr]
    // put IPPROTO_MAX later
    use c::{
        AF_DECnet, AF_APPLETALK, AF_INET, AF_INET6, AF_IPX, AF_UNSPEC, INADDR_ANY, INADDR_LOOPBACK,
        INADDR_NONE, IPPROTO_AH, IPPROTO_DSTOPTS, IPPROTO_EGP, IPPROTO_ESP, IPPROTO_FRAGMENT,
        IPPROTO_HOPOPTS, IPPROTO_ICMP, IPPROTO_ICMPV6, IPPROTO_IDP, IPPROTO_IGMP, IPPROTO_IP,
        IPPROTO_IP as IPPROTO_IPIP, IPPROTO_IPV6, IPPROTO_NONE, IPPROTO_PIM, IPPROTO_PUP,
        IPPROTO_RAW, IPPROTO_ROUTING, IPPROTO_TCP, IPPROTO_TCP as SOL_TCP, IPPROTO_UDP, MSG_CTRUNC,
        MSG_DONTROUTE, MSG_OOB, MSG_PEEK, MSG_TRUNC, MSG_WAITALL, NI_DGRAM, NI_MAXHOST,
        NI_NAMEREQD, NI_NOFQDN, NI_NUMERICHOST, NI_NUMERICSERV, SHUT_RD, SHUT_RDWR, SHUT_WR,
        SOCK_DGRAM, SOCK_STREAM, SOL_SOCKET, SO_BROADCAST, SO_ERROR, SO_LINGER, SO_OOBINLINE,
        SO_REUSEADDR, SO_TYPE, TCP_NODELAY,
    };

    #[cfg(unix)]
    #[pyattr]
    use c::{AF_UNIX, SO_REUSEPORT};

    #[pyattr]
    use c::{AI_ADDRCONFIG, AI_NUMERICHOST, AI_NUMERICSERV, AI_PASSIVE};

    #[cfg(not(target_os = "redox"))]
    #[pyattr]
    use c::{SOCK_RAW, SOCK_RDM, SOCK_SEQPACKET};

    #[cfg(target_os = "android")]
    #[pyattr]
    use c::{SOL_ATALK, SOL_AX25, SOL_IPX, SOL_NETROM, SOL_ROSE};

    #[cfg(target_os = "freebsd")]
    #[pyattr]
    use c::SO_SETFIB;

    #[cfg(target_os = "linux")]
    #[pyattr]
    use c::{
        CAN_BCM, CAN_EFF_FLAG, CAN_EFF_MASK, CAN_ERR_FLAG, CAN_ERR_MASK, CAN_ISOTP, CAN_J1939,
        CAN_RAW, CAN_RAW_ERR_FILTER, CAN_RAW_FD_FRAMES, CAN_RAW_FILTER, CAN_RAW_JOIN_FILTERS,
        CAN_RAW_LOOPBACK, CAN_RAW_RECV_OWN_MSGS, CAN_RTR_FLAG, CAN_SFF_MASK, IPPROTO_MPTCP,
        J1939_IDLE_ADDR, J1939_MAX_UNICAST_ADDR, J1939_NLA_BYTES_ACKED, J1939_NLA_PAD,
        J1939_NO_ADDR, J1939_NO_NAME, J1939_NO_PGN, J1939_PGN_ADDRESS_CLAIMED,
        J1939_PGN_ADDRESS_COMMANDED, J1939_PGN_MAX, J1939_PGN_PDU1_MAX, J1939_PGN_REQUEST,
        SCM_J1939_DEST_ADDR, SCM_J1939_DEST_NAME, SCM_J1939_ERRQUEUE, SCM_J1939_PRIO, SOL_CAN_BASE,
        SOL_CAN_RAW, SO_J1939_ERRQUEUE, SO_J1939_FILTER, SO_J1939_PROMISC, SO_J1939_SEND_PRIO,
    };

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[pyattr]
    use c::SOL_RDS;

    #[cfg(target_os = "netbsd")]
    #[pyattr]
    use c::IPPROTO_VRRP;

    #[cfg(target_vendor = "apple")]
    #[pyattr]
    use c::{AF_SYSTEM, PF_SYSTEM, SYSPROTO_CONTROL, TCP_KEEPALIVE};

    #[cfg(windows)]
    #[pyattr]
    use c::{
        IPPORT_RESERVED, IPPROTO_IPV4, RCVALL_IPLEVEL, RCVALL_OFF, RCVALL_ON,
        RCVALL_SOCKETLEVELONLY, SIO_KEEPALIVE_VALS, SIO_LOOPBACK_FAST_PATH, SIO_RCVALL,
        SO_EXCLUSIVEADDRUSE,
    };

    #[cfg(not(windows))]
    #[pyattr]
    const IPPORT_RESERVED: i32 = 1024;

    #[pyattr]
    const IPPORT_USERRESERVED: i32 = 5000;

    #[cfg(any(unix, target_os = "android"))]
    #[pyattr]
    use c::{
        EAI_SYSTEM, MSG_EOR, SO_ACCEPTCONN, SO_DEBUG, SO_DONTROUTE, SO_KEEPALIVE, SO_RCVBUF,
        SO_RCVLOWAT, SO_RCVTIMEO, SO_SNDBUF, SO_SNDLOWAT, SO_SNDTIMEO,
    };

    #[cfg(any(target_os = "android", target_os = "linux"))]
    #[pyattr]
    use c::{
        ALG_OP_DECRYPT, ALG_OP_ENCRYPT, ALG_SET_AEAD_ASSOCLEN, ALG_SET_AEAD_AUTHSIZE, ALG_SET_IV,
        ALG_SET_KEY, ALG_SET_OP, IPV6_DSTOPTS, IPV6_NEXTHOP, IPV6_PATHMTU, IPV6_RECVDSTOPTS,
        IPV6_RECVHOPLIMIT, IPV6_RECVHOPOPTS, IPV6_RECVPATHMTU, IPV6_RTHDRDSTOPTS,
        IP_DEFAULT_MULTICAST_LOOP, IP_RECVOPTS, IP_RETOPTS, NETLINK_CRYPTO, NETLINK_DNRTMSG,
        NETLINK_FIREWALL, NETLINK_IP6_FW, NETLINK_NFLOG, NETLINK_ROUTE, NETLINK_USERSOCK,
        NETLINK_XFRM, SOL_ALG, SO_PASSSEC, SO_PEERSEC,
    };

    #[cfg(any(target_os = "android", target_vendor = "apple"))]
    #[pyattr]
    use c::{AI_DEFAULT, AI_MASK, AI_V4MAPPED_CFG};

    #[cfg(any(target_os = "freebsd", target_os = "netbsd"))]
    #[pyattr]
    use c::MSG_NOTIFICATION;

    #[cfg(any(target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    use c::TCP_USER_TIMEOUT;

    #[cfg(any(unix, target_os = "android", windows))]
    #[pyattr]
    use c::{
        INADDR_BROADCAST, IPV6_MULTICAST_HOPS, IPV6_MULTICAST_IF, IPV6_MULTICAST_LOOP,
        IPV6_UNICAST_HOPS, IPV6_V6ONLY, IP_ADD_MEMBERSHIP, IP_DROP_MEMBERSHIP, IP_MULTICAST_IF,
        IP_MULTICAST_LOOP, IP_MULTICAST_TTL, IP_TTL,
    };

    #[cfg(any(unix, target_os = "android", windows))]
    #[pyattr]
    const INADDR_UNSPEC_GROUP: u32 = 0xe0000000;

    #[cfg(any(unix, target_os = "android", windows))]
    #[pyattr]
    const INADDR_ALLHOSTS_GROUP: u32 = 0xe0000001;

    #[cfg(any(unix, target_os = "android", windows))]
    #[pyattr]
    const INADDR_MAX_LOCAL_GROUP: u32 = 0xe00000ff;

    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    use c::{
        AF_ALG, AF_ASH, AF_ATMPVC, AF_ATMSVC, AF_AX25, AF_BRIDGE, AF_CAN, AF_ECONET, AF_IRDA,
        AF_LLC, AF_NETBEUI, AF_NETLINK, AF_NETROM, AF_PACKET, AF_PPPOX, AF_RDS, AF_SECURITY,
        AF_TIPC, AF_VSOCK, AF_WANPIPE, AF_X25, IP_TRANSPARENT, MSG_CONFIRM, MSG_ERRQUEUE,
        MSG_FASTOPEN, MSG_MORE, PF_CAN, PF_PACKET, PF_RDS, SCM_CREDENTIALS, SOL_IP, SOL_TIPC,
        SOL_UDP, SO_BINDTODEVICE, SO_MARK, TCP_CORK, TCP_DEFER_ACCEPT, TCP_LINGER2, TCP_QUICKACK,
        TCP_SYNCNT, TCP_WINDOW_CLAMP,
    };

    // gated on presence of AF_VSOCK:
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    const SO_VM_SOCKETS_BUFFER_SIZE: u32 = 0;

    // gated on presence of AF_VSOCK:
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    const SO_VM_SOCKETS_BUFFER_MIN_SIZE: u32 = 1;

    // gated on presence of AF_VSOCK:
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    const SO_VM_SOCKETS_BUFFER_MAX_SIZE: u32 = 2;

    // gated on presence of AF_VSOCK:
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    const VMADDR_CID_ANY: u32 = 0xffffffff; // 0xffffffff

    // gated on presence of AF_VSOCK:
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    const VMADDR_PORT_ANY: u32 = 0xffffffff; // 0xffffffff

    // gated on presence of AF_VSOCK:
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    const VMADDR_CID_HOST: u32 = 2;

    // gated on presence of AF_VSOCK:
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    const VM_SOCKETS_INVALID_VERSION: u32 = 0xffffffff; // 0xffffffff

    // TODO: gated on https://github.com/rust-lang/libc/pull/1662
    // // gated on presence of AF_VSOCK:
    // #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    // #[pyattr(name = "IOCTL_VM_SOCKETS_GET_LOCAL_CID", once)]
    // fn ioctl_vm_sockets_get_local_cid(_vm: &VirtualMachine) -> i32 {
    //     c::_IO(7, 0xb9)
    // }

    #[cfg(not(any(target_os = "android", target_os = "fuchsia", target_os = "linux")))]
    #[pyattr]
    const SOL_IP: i32 = 0;

    #[cfg(not(any(target_os = "android", target_os = "fuchsia", target_os = "linux")))]
    #[pyattr]
    const SOL_UDP: i32 = 17;

    #[cfg(any(target_os = "android", target_os = "linux", windows))]
    #[pyattr]
    use c::{IPV6_HOPOPTS, IPV6_RECVRTHDR, IPV6_RTHDR, IP_OPTIONS};

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_vendor = "apple"
    ))]
    #[pyattr]
    use c::{IPPROTO_HELLO, IPPROTO_XTP, LOCAL_PEERCRED, MSG_EOF};

    #[cfg(any(target_os = "netbsd", target_os = "openbsd", windows))]
    #[pyattr]
    use c::{MSG_BCAST, MSG_MCAST};

    #[cfg(any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "freebsd",
        target_os = "linux"
    ))]
    #[pyattr]
    use c::{IPPROTO_UDPLITE, TCP_CONGESTION};

    #[cfg(any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "freebsd",
        target_os = "linux"
    ))]
    #[pyattr]
    const UDPLITE_SEND_CSCOV: i32 = 10;

    #[cfg(any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "freebsd",
        target_os = "linux"
    ))]
    #[pyattr]
    const UDPLITE_RECV_CSCOV: i32 = 11;

    #[cfg(any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use c::AF_KEY;

    #[cfg(any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "redox"
    ))]
    #[pyattr]
    use c::SO_DOMAIN;

    #[cfg(any(
        target_os = "android",
        target_os = "fuchsia",
        all(
            target_os = "linux",
            any(
                target_arch = "aarch64",
                target_arch = "i686",
                target_arch = "mips",
                target_arch = "powerpc",
                target_arch = "powerpc64",
                target_arch = "powerpc64le",
                target_arch = "riscv64gc",
                target_arch = "s390x",
                target_arch = "x86_64"
            )
        ),
        target_os = "redox"
    ))]
    #[pyattr]
    use c::SO_PRIORITY;

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use c::IPPROTO_MOBILE;

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_vendor = "apple"
    ))]
    #[pyattr]
    use c::SCM_CREDS;

    #[cfg(any(
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_vendor = "apple"
    ))]
    #[pyattr]
    use c::TCP_FASTOPEN;

    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "fuchsia",
        all(
            target_os = "linux",
            any(
                target_arch = "aarch64",
                target_arch = "i686",
                target_arch = "mips",
                target_arch = "powerpc",
                target_arch = "powerpc64",
                target_arch = "powerpc64le",
                target_arch = "riscv64gc",
                target_arch = "s390x",
                target_arch = "x86_64"
            )
        ),
        target_os = "redox"
    ))]
    #[pyattr]
    use c::SO_PROTOCOL;

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        windows
    ))]
    #[pyattr]
    use c::IPV6_DONTFRAG;

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "redox"
    ))]
    #[pyattr]
    use c::{SO_PASSCRED, SO_PEERCRED};

    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "netbsd"
    ))]
    #[pyattr]
    use c::TCP_INFO;

    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_vendor = "apple"
    ))]
    #[pyattr]
    use c::IP_RECVTOS;

    #[cfg(any(
        target_os = "android",
        target_os = "netbsd",
        target_os = "redox",
        target_vendor = "apple",
        windows
    ))]
    #[pyattr]
    use c::NI_MAXSERV;

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_vendor = "apple"
    ))]
    #[pyattr]
    use c::{IPPROTO_EON, IPPROTO_IPCOMP};

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_vendor = "apple",
        windows
    ))]
    #[pyattr]
    use c::IPPROTO_ND;

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_vendor = "apple",
        windows
    ))]
    #[pyattr]
    use c::{IPV6_CHECKSUM, IPV6_HOPLIMIT};

    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "netbsd"
    ))]
    #[pyattr]
    use c::IPPROTO_SCTP; // also in windows

    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_vendor = "apple",
        windows
    ))]
    #[pyattr]
    use c::{AI_ALL, AI_V4MAPPED};

    #[cfg(any(
        target_os = "android",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd",
        target_vendor = "apple",
        windows
    ))]
    #[pyattr]
    use c::EAI_NODATA;

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_vendor = "apple",
        windows
    ))]
    #[pyattr]
    use c::{
        AF_LINK, IPPROTO_GGP, IPV6_JOIN_GROUP, IPV6_LEAVE_GROUP, IP_RECVDSTADDR, SO_USELOOPBACK,
    };

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use c::{MSG_CMSG_CLOEXEC, MSG_NOSIGNAL};

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "redox"
    ))]
    #[pyattr]
    use c::TCP_KEEPIDLE;

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "netbsd",
        target_vendor = "apple"
    ))]
    #[pyattr]
    use c::{TCP_KEEPCNT, TCP_KEEPINTVL};

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "redox"
    ))]
    #[pyattr]
    use c::{SOCK_CLOEXEC, SOCK_NONBLOCK};

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd",
        target_vendor = "apple"
    ))]
    #[pyattr]
    use c::{
        AF_ROUTE, AF_SNA, EAI_OVERFLOW, IPPROTO_GRE, IPPROTO_RSVP, IPPROTO_TP, IPV6_RECVPKTINFO,
        MSG_DONTWAIT, SCM_RIGHTS, TCP_MAXSEG,
    };

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd",
        target_vendor = "apple",
        windows
    ))]
    #[pyattr]
    use c::IPV6_PKTINFO;

    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd",
        target_vendor = "apple",
        windows
    ))]
    #[pyattr]
    use c::AI_CANONNAME;

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd",
        target_vendor = "apple",
        windows
    ))]
    #[pyattr]
    use c::{
        EAI_AGAIN, EAI_BADFLAGS, EAI_FAIL, EAI_FAMILY, EAI_MEMORY, EAI_NONAME, EAI_SERVICE,
        EAI_SOCKTYPE, IPV6_RECVTCLASS, IPV6_TCLASS, IP_HDRINCL, IP_TOS, SOMAXCONN,
    };

    #[cfg(not(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd",
        target_vendor = "apple",
        windows
    )))]
    #[pyattr]
    const SOMAXCONN: i32 = 5; // Common value

    // HERE IS WHERE THE BLUETOOTH CONSTANTS START
    // TODO: there should be a more intelligent way of detecting bluetooth on a platform.
    //       CPython uses header-detection, but blocks NetBSD and DragonFly BSD
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use c::AF_BLUETOOTH;

    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyattr]
    const BDADDR_ANY: &str = "00:00:00:00:00:00";
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyattr]
    const BDADDR_LOCAL: &str = "00:00:00:FF:FF:FF";
    // HERE IS WHERE THE BLUETOOTH CONSTANTS END

    #[cfg(windows)]
    #[pyattr]
    use winapi::shared::ws2def::{
        IPPROTO_CBT, IPPROTO_ICLFXBM, IPPROTO_IGP, IPPROTO_L2TP, IPPROTO_PGM, IPPROTO_RDP,
        IPPROTO_SCTP, IPPROTO_ST,
    };

    #[pyattr]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.os_error.to_owned()
    }

    #[pyattr]
    fn timeout(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.timeout_error.to_owned()
    }

    #[pyattr(once)]
    fn herror(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "socket",
            "herror",
            Some(vec![vm.ctx.exceptions.os_error.to_owned()]),
        )
    }
    #[pyattr(once)]
    fn gaierror(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "socket",
            "gaierror",
            Some(vec![vm.ctx.exceptions.os_error.to_owned()]),
        )
    }

    #[pyfunction]
    fn htonl(x: u32) -> u32 {
        u32::to_be(x)
    }
    #[pyfunction]
    fn htons(x: u16) -> u16 {
        u16::to_be(x)
    }
    #[pyfunction]
    fn ntohl(x: u32) -> u32 {
        u32::from_be(x)
    }
    #[pyfunction]
    fn ntohs(x: u16) -> u16 {
        u16::from_be(x)
    }

    #[cfg(unix)]
    type RawSocket = std::os::unix::io::RawFd;
    #[cfg(windows)]
    type RawSocket = std::os::windows::raw::SOCKET;

    #[cfg(unix)]
    macro_rules! errcode {
        ($e:ident) => {
            c::$e
        };
    }
    #[cfg(windows)]
    macro_rules! errcode {
    ($e:ident) => {
        paste::paste!(c::[<WSA $e>])
    };
}

    #[cfg(windows)]
    use winapi::shared::netioapi;

    fn get_raw_sock(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<RawSocket> {
        #[cfg(unix)]
        type CastFrom = libc::c_long;
        #[cfg(windows)]
        type CastFrom = libc::c_longlong;

        // should really just be to_index() but test_socket tests the error messages explicitly
        if obj.fast_isinstance(vm.ctx.types.float_type) {
            return Err(vm.new_type_error("integer argument expected, got float".to_owned()));
        }
        let int = obj
            .try_index_opt(vm)
            .unwrap_or_else(|| Err(vm.new_type_error("an integer is required".to_owned())))?;
        int.try_to_primitive::<CastFrom>(vm)
            .map(|sock| sock as RawSocket)
    }

    #[pyattr(name = "socket")]
    #[pyattr(name = "SocketType")]
    #[pyclass(name = "socket")]
    #[derive(Debug, PyPayload)]
    pub struct PySocket {
        kind: AtomicCell<i32>,
        family: AtomicCell<i32>,
        proto: AtomicCell<i32>,
        pub(crate) timeout: AtomicCell<f64>,
        sock: PyRwLock<Option<Socket>>,
    }

    const _: () = assert!(std::mem::size_of::<Option<Socket>>() == std::mem::size_of::<Socket>());

    impl Default for PySocket {
        fn default() -> Self {
            PySocket {
                kind: AtomicCell::default(),
                family: AtomicCell::default(),
                proto: AtomicCell::default(),
                timeout: AtomicCell::new(-1.0),
                sock: PyRwLock::new(None),
            }
        }
    }

    #[cfg(windows)]
    const CLOSED_ERR: i32 = c::WSAENOTSOCK;
    #[cfg(unix)]
    const CLOSED_ERR: i32 = c::EBADF;

    impl Read for &PySocket {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            (&mut &*self.sock()?).read(buf)
        }
    }
    impl Write for &PySocket {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            (&mut &*self.sock()?).write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            (&mut &*self.sock()?).flush()
        }
    }

    impl PySocket {
        pub fn sock_opt(&self) -> Option<PyMappedRwLockReadGuard<'_, Socket>> {
            PyRwLockReadGuard::try_map(self.sock.read(), |sock| sock.as_ref()).ok()
        }

        pub fn sock(&self) -> io::Result<PyMappedRwLockReadGuard<'_, Socket>> {
            self.sock_opt()
                .ok_or_else(|| io::Error::from_raw_os_error(CLOSED_ERR))
        }

        fn init_inner(
            &self,
            family: i32,
            socket_kind: i32,
            proto: i32,
            sock: Socket,
        ) -> io::Result<()> {
            self.family.store(family);
            self.kind.store(socket_kind);
            self.proto.store(proto);
            let mut s = self.sock.write();
            let sock = s.insert(sock);
            let timeout = DEFAULT_TIMEOUT.load();
            self.timeout.store(timeout);
            if timeout >= 0.0 {
                sock.set_nonblocking(true)?;
            }
            Ok(())
        }

        /// returns Err(blocking)
        pub fn get_timeout(&self) -> Result<Duration, bool> {
            let timeout = self.timeout.load();
            if timeout > 0.0 {
                Ok(Duration::from_secs_f64(timeout))
            } else {
                Err(timeout != 0.0)
            }
        }

        fn sock_op<F, R>(
            &self,
            vm: &VirtualMachine,
            select: SelectKind,
            f: F,
        ) -> Result<R, IoOrPyException>
        where
            F: FnMut() -> io::Result<R>,
        {
            self.sock_op_timeout_err(vm, select, self.get_timeout().ok(), f)
        }

        fn sock_op_timeout_err<F, R>(
            &self,
            vm: &VirtualMachine,
            select: SelectKind,
            timeout: Option<Duration>,
            mut f: F,
        ) -> Result<R, IoOrPyException>
        where
            F: FnMut() -> io::Result<R>,
        {
            let deadline = timeout.map(Deadline::new);

            loop {
                if deadline.is_some() || matches!(select, SelectKind::Connect) {
                    let interval = deadline.as_ref().map(|d| d.time_until()).transpose()?;
                    let res = sock_select(&*self.sock()?, select, interval);
                    match res {
                        Ok(true) => return Err(IoOrPyException::Timeout),
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                            vm.check_signals()?;
                            continue;
                        }
                        Err(e) => return Err(e.into()),
                        Ok(false) => {} // no timeout, continue as normal
                    }
                }

                let err = loop {
                    // loop on interrupt
                    match f() {
                        Ok(x) => return Ok(x),
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => vm.check_signals()?,
                        Err(e) => break e,
                    }
                };
                if timeout.is_some() && err.kind() == io::ErrorKind::WouldBlock {
                    continue;
                }
                return Err(err.into());
            }
        }

        fn extract_address(
            &self,
            addr: PyObjectRef,
            caller: &str,
            vm: &VirtualMachine,
        ) -> Result<socket2::SockAddr, IoOrPyException> {
            let family = self.family.load();
            match family {
                #[cfg(unix)]
                c::AF_UNIX => {
                    use std::os::unix::ffi::OsStrExt;
                    let buf = crate::vm::function::ArgStrOrBytesLike::try_from_object(vm, addr)?;
                    let path = &*buf.borrow_bytes();
                    if cfg!(any(target_os = "linux", target_os = "android"))
                        && path.first() == Some(&0)
                    {
                        use libc::{sa_family_t, socklen_t};
                        use {socket2::SockAddr, std::ptr};
                        unsafe {
                            // based on SockAddr::unix
                            // TODO: upstream or fix socklen check for SockAddr::unix()?
                            SockAddr::init(|storage, len| {
                                // Safety: `SockAddr::init` zeros the address, which is a valid
                                // representation.
                                let storage: &mut libc::sockaddr_un = &mut *storage.cast();
                                let len: &mut socklen_t = &mut *len;

                                let bytes = path;
                                if bytes.len() > storage.sun_path.len() {
                                    return Err(io::Error::new(
                                        io::ErrorKind::InvalidInput,
                                        "path must be shorter than SUN_LEN",
                                    ));
                                }

                                storage.sun_family = libc::AF_UNIX as sa_family_t;
                                // Safety: `bytes` and `addr.sun_path` are not overlapping and
                                // both point to valid memory.
                                // `SockAddr::init` zeroes the memory, so the path is already
                                // null terminated.
                                ptr::copy_nonoverlapping(
                                    bytes.as_ptr(),
                                    storage.sun_path.as_mut_ptr() as *mut u8,
                                    bytes.len(),
                                );

                                let base = storage as *const _ as usize;
                                let path = &storage.sun_path as *const _ as usize;
                                let sun_path_offset = path - base;
                                let length = sun_path_offset + bytes.len();
                                *len = length as socklen_t;

                                Ok(())
                            })
                        }
                        .map(|(_, addr)| addr)
                    } else {
                        socket2::SockAddr::unix(ffi::OsStr::from_bytes(path))
                    }
                    .map_err(|_| vm.new_os_error("AF_UNIX path too long".to_owned()).into())
                }
                c::AF_INET => {
                    let tuple: PyTupleRef = addr.downcast().map_err(|obj| {
                        vm.new_type_error(format!(
                            "{}(): AF_INET address must be tuple, not {}",
                            caller,
                            obj.class().name()
                        ))
                    })?;
                    if tuple.len() != 2 {
                        return Err(vm
                            .new_type_error(
                                "AF_INET address must be a pair (host, post)".to_owned(),
                            )
                            .into());
                    }
                    let addr = Address::from_tuple(&tuple, vm)?;
                    let mut addr4 = get_addr(vm, addr.host, c::AF_INET)?;
                    match &mut addr4 {
                        SocketAddr::V4(addr4) => {
                            addr4.set_port(addr.port);
                        }
                        SocketAddr::V6(_) => unreachable!(),
                    }
                    Ok(addr4.into())
                }
                c::AF_INET6 => {
                    let tuple: PyTupleRef = addr.downcast().map_err(|obj| {
                        vm.new_type_error(format!(
                            "{}(): AF_INET6 address must be tuple, not {}",
                            caller,
                            obj.class().name()
                        ))
                    })?;
                    match tuple.len() {
                        2 | 3 | 4 => {}
                        _ => return Err(vm.new_type_error(
                            "AF_INET6 address must be a tuple (host, port[, flowinfo[, scopeid]])"
                                .to_owned(),
                        ).into()),
                    }
                    let (addr, flowinfo, scopeid) = Address::from_tuple_ipv6(&tuple, vm)?;
                    let mut addr6 = get_addr(vm, addr.host, c::AF_INET6)?;
                    match &mut addr6 {
                        SocketAddr::V6(addr6) => {
                            addr6.set_port(addr.port);
                            addr6.set_flowinfo(flowinfo);
                            addr6.set_scope_id(scopeid);
                        }
                        SocketAddr::V4(_) => unreachable!(),
                    }
                    Ok(addr6.into())
                }
                _ => Err(vm.new_os_error(format!("{}(): bad family", caller)).into()),
            }
        }

        fn connect_inner(
            &self,
            address: PyObjectRef,
            caller: &str,
            vm: &VirtualMachine,
        ) -> Result<(), IoOrPyException> {
            let sock_addr = self.extract_address(address, caller, vm)?;

            let err = match self.sock()?.connect(&sock_addr) {
                Ok(()) => return Ok(()),
                Err(e) => e,
            };

            let wait_connect = if err.kind() == io::ErrorKind::Interrupted {
                vm.check_signals()?;
                self.timeout.load() != 0.0
            } else {
                #[cfg(unix)]
                use c::EINPROGRESS;
                #[cfg(windows)]
                use c::WSAEWOULDBLOCK as EINPROGRESS;

                self.timeout.load() > 0.0 && err.raw_os_error() == Some(EINPROGRESS)
            };

            if wait_connect {
                // basically, connect() is async, and it registers an "error" on the socket when it's
                // done connecting. SelectKind::Connect fills the errorfds fd_set, so if we wake up
                // from poll and the error is EISCONN then we know that the connect is done
                self.sock_op(vm, SelectKind::Connect, || {
                    let sock = self.sock()?;
                    let err = sock.take_error()?;
                    match err {
                        Some(e) if e.raw_os_error() == Some(libc::EISCONN) => Ok(()),
                        Some(e) => Err(e),
                        // TODO: is this accurate?
                        None => Ok(()),
                    }
                })
            } else {
                Err(err.into())
            }
        }
    }

    impl DefaultConstructor for PySocket {}

    impl Initializer for PySocket {
        type Args = (
            OptionalArg<i32>,
            OptionalArg<i32>,
            OptionalArg<i32>,
            OptionalOption<PyObjectRef>,
        );

        fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            Self::_init(zelf, args, vm).map_err(|e| e.into_pyexception(vm))
        }
    }

    #[pyclass(with(DefaultConstructor, Initializer), flags(BASETYPE))]
    impl PySocket {
        fn _init(
            zelf: PyRef<Self>,
            (family, socket_kind, proto, fileno): <Self as Initializer>::Args,
            vm: &VirtualMachine,
        ) -> Result<(), IoOrPyException> {
            let mut family = family.unwrap_or(-1);
            let mut socket_kind = socket_kind.unwrap_or(-1);
            let mut proto = proto.unwrap_or(-1);
            let fileno = fileno
                .flatten()
                .map(|obj| get_raw_sock(obj, vm))
                .transpose()?;
            let sock;
            if let Some(fileno) = fileno {
                sock = sock_from_raw(fileno, vm)?;
                match sock.local_addr() {
                    Ok(addr) if family == -1 => family = addr.family() as i32,
                    Err(e)
                        if family == -1
                            || matches!(
                                e.raw_os_error(),
                                Some(errcode!(ENOTSOCK)) | Some(errcode!(EBADF))
                            ) =>
                    {
                        std::mem::forget(sock);
                        return Err(e.into());
                    }
                    _ => {}
                }
                if socket_kind == -1 {
                    // TODO: when socket2 cuts a new release, type will be available on all os
                    // socket_kind = sock.r#type().map_err(|e| e.into_pyexception(vm))?.into();
                    let res = unsafe {
                        c::getsockopt(
                            sock_fileno(&sock) as _,
                            c::SOL_SOCKET,
                            c::SO_TYPE,
                            &mut socket_kind as *mut libc::c_int as *mut _,
                            &mut (std::mem::size_of::<i32>() as _),
                        )
                    };
                    if res < 0 {
                        return Err(crate::common::os::errno().into());
                    }
                }
                cfg_if::cfg_if! {
                    if #[cfg(any(
                        target_os = "android",
                        target_os = "freebsd",
                        target_os = "fuchsia",
                        target_os = "linux",
                    ))] {
                        if proto == -1 {
                            proto = sock.protocol()?.map_or(0, Into::into);
                        }
                    } else {
                        proto = 0;
                    }
                }
            } else {
                if family == -1 {
                    family = c::AF_INET as i32
                }
                if socket_kind == -1 {
                    socket_kind = c::SOCK_STREAM
                }
                if proto == -1 {
                    proto = 0
                }
                sock = Socket::new(
                    Domain::from(family),
                    SocketType::from(socket_kind),
                    Some(Protocol::from(proto)),
                )?;
            };
            Ok(zelf.init_inner(family, socket_kind, proto, sock)?)
        }

        #[pymethod]
        fn connect(
            &self,
            address: PyObjectRef,
            vm: &VirtualMachine,
        ) -> Result<(), IoOrPyException> {
            self.connect_inner(address, "connect", vm)
        }

        #[pymethod]
        fn connect_ex(&self, address: PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
            match self.connect_inner(address, "connect_ex", vm) {
                Ok(()) => Ok(0),
                Err(err) => err.errno(),
            }
        }

        #[pymethod]
        fn bind(&self, address: PyObjectRef, vm: &VirtualMachine) -> Result<(), IoOrPyException> {
            let sock_addr = self.extract_address(address, "bind", vm)?;
            Ok(self.sock()?.bind(&sock_addr)?)
        }

        #[pymethod]
        fn listen(&self, backlog: OptionalArg<i32>) -> io::Result<()> {
            let backlog = backlog.unwrap_or(128);
            let backlog = if backlog < 0 { 0 } else { backlog };
            self.sock()?.listen(backlog)
        }

        #[pymethod]
        fn _accept(
            &self,
            vm: &VirtualMachine,
        ) -> Result<(RawSocket, PyObjectRef), IoOrPyException> {
            let (sock, addr) = self.sock_op(vm, SelectKind::Read, || self.sock()?.accept())?;
            let fd = into_sock_fileno(sock);
            Ok((fd, get_addr_tuple(&addr, vm)))
        }

        #[pymethod]
        fn recv(
            &self,
            bufsize: usize,
            flags: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> Result<Vec<u8>, IoOrPyException> {
            let flags = flags.unwrap_or(0);
            let mut buffer = Vec::with_capacity(bufsize);
            let sock = self.sock()?;
            let n = self.sock_op(vm, SelectKind::Read, || {
                sock.recv_with_flags(buffer.spare_capacity_mut(), flags)
            })?;
            unsafe { buffer.set_len(n) };
            Ok(buffer)
        }

        #[pymethod]
        fn recv_into(
            &self,
            buf: ArgMemoryBuffer,
            flags: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> Result<usize, IoOrPyException> {
            let flags = flags.unwrap_or(0);
            let sock = self.sock()?;
            let mut buf = buf.borrow_buf_mut();
            let buf = &mut *buf;
            self.sock_op(vm, SelectKind::Read, || {
                sock.recv_with_flags(slice_as_uninit(buf), flags)
            })
        }

        #[pymethod]
        fn recvfrom(
            &self,
            bufsize: isize,
            flags: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> Result<(Vec<u8>, PyObjectRef), IoOrPyException> {
            let flags = flags.unwrap_or(0);
            let bufsize = bufsize
                .to_usize()
                .ok_or_else(|| vm.new_value_error("negative buffersize in recvfrom".to_owned()))?;
            let mut buffer = Vec::with_capacity(bufsize);
            let (n, addr) = self.sock_op(vm, SelectKind::Read, || {
                self.sock()?
                    .recv_from_with_flags(buffer.spare_capacity_mut(), flags)
            })?;
            unsafe { buffer.set_len(n) };
            Ok((buffer, get_addr_tuple(&addr, vm)))
        }

        #[pymethod]
        fn recvfrom_into(
            &self,
            buf: ArgMemoryBuffer,
            nbytes: OptionalArg<isize>,
            flags: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> Result<(usize, PyObjectRef), IoOrPyException> {
            let mut buf = buf.borrow_buf_mut();
            let buf = &mut *buf;
            let buf = match nbytes {
                OptionalArg::Present(i) => {
                    let i = i.to_usize().ok_or_else(|| {
                        vm.new_value_error("negative buffersize in recvfrom_into".to_owned())
                    })?;
                    buf.get_mut(..i).ok_or_else(|| {
                        vm.new_value_error(
                            "nbytes is greater than the length of the buffer".to_owned(),
                        )
                    })?
                }
                OptionalArg::Missing => buf,
            };
            let flags = flags.unwrap_or(0);
            let sock = self.sock()?;
            let (n, addr) = self.sock_op(vm, SelectKind::Read, || {
                sock.recv_from_with_flags(slice_as_uninit(buf), flags)
            })?;
            Ok((n, get_addr_tuple(&addr, vm)))
        }

        #[pymethod]
        fn send(
            &self,
            bytes: ArgBytesLike,
            flags: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> Result<usize, IoOrPyException> {
            let flags = flags.unwrap_or(0);
            let buf = bytes.borrow_buf();
            let buf = &*buf;
            self.sock_op(vm, SelectKind::Write, || {
                self.sock()?.send_with_flags(buf, flags)
            })
        }

        #[pymethod]
        fn sendall(
            &self,
            bytes: ArgBytesLike,
            flags: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> Result<(), IoOrPyException> {
            let flags = flags.unwrap_or(0);

            let timeout = self.get_timeout().ok();

            let deadline = timeout.map(Deadline::new);

            let buf = bytes.borrow_buf();
            let buf = &*buf;
            let mut buf_offset = 0;
            // now we have like 3 layers of interrupt loop :)
            while buf_offset < buf.len() {
                let interval = deadline.as_ref().map(|d| d.time_until()).transpose()?;
                self.sock_op_timeout_err(vm, SelectKind::Write, interval, || {
                    let subbuf = &buf[buf_offset..];
                    buf_offset += self.sock()?.send_with_flags(subbuf, flags)?;
                    Ok(())
                })?;
                vm.check_signals()?;
            }
            Ok(())
        }

        #[pymethod]
        fn sendto(
            &self,
            bytes: ArgBytesLike,
            arg2: PyObjectRef,
            arg3: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> Result<usize, IoOrPyException> {
            // signature is bytes[, flags], address
            let (flags, address) = match arg3 {
                OptionalArg::Present(arg3) => {
                    // should just be i32::try_from_obj but tests check for error message
                    let int = arg2.try_index_opt(vm).unwrap_or_else(|| {
                        Err(vm.new_type_error("an integer is required".to_owned()))
                    })?;
                    let flags = int.try_to_primitive::<i32>(vm)?;
                    (flags, arg3)
                }
                OptionalArg::Missing => (0, arg2),
            };
            let addr = self.extract_address(address, "sendto", vm)?;
            let buf = bytes.borrow_buf();
            let buf = &*buf;
            self.sock_op(vm, SelectKind::Write, || {
                self.sock()?.send_to_with_flags(buf, &addr, flags)
            })
        }

        #[pymethod]
        fn close(&self) -> io::Result<()> {
            let sock = self.detach();
            if sock != INVALID_SOCKET {
                close_inner(sock)?;
            }
            Ok(())
        }
        #[pymethod]
        #[inline]
        fn detach(&self) -> RawSocket {
            let sock = self.sock.write().take();
            sock.map_or(INVALID_SOCKET, into_sock_fileno)
        }

        #[pymethod]
        fn fileno(&self) -> RawSocket {
            self.sock
                .read()
                .as_ref()
                .map_or(INVALID_SOCKET, sock_fileno)
        }

        #[pymethod]
        fn getsockname(&self, vm: &VirtualMachine) -> std::io::Result<PyObjectRef> {
            let addr = self.sock()?.local_addr()?;

            Ok(get_addr_tuple(&addr, vm))
        }
        #[pymethod]
        fn getpeername(&self, vm: &VirtualMachine) -> std::io::Result<PyObjectRef> {
            let addr = self.sock()?.peer_addr()?;

            Ok(get_addr_tuple(&addr, vm))
        }

        #[pymethod]
        fn gettimeout(&self) -> Option<f64> {
            let timeout = self.timeout.load();
            if timeout >= 0.0 {
                Some(timeout)
            } else {
                None
            }
        }

        #[pymethod]
        fn setblocking(&self, block: bool) -> io::Result<()> {
            self.timeout.store(if block { -1.0 } else { 0.0 });
            self.sock()?.set_nonblocking(!block)
        }

        #[pymethod]
        fn getblocking(&self) -> bool {
            self.timeout.load() != 0.0
        }

        #[pymethod]
        fn settimeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.timeout
                .store(timeout.map_or(-1.0, |d| d.as_secs_f64()));
            // even if timeout is > 0 the socket needs to be nonblocking in order for us to select() on
            // it
            self.sock()?.set_nonblocking(timeout.is_some())
        }

        #[pymethod]
        fn getsockopt(
            &self,
            level: i32,
            name: i32,
            buflen: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> Result<PyObjectRef, IoOrPyException> {
            let sock = self.sock()?;
            let fd = sock_fileno(&sock);
            let buflen = buflen.unwrap_or(0);
            if buflen == 0 {
                let mut flag: libc::c_int = 0;
                let mut flagsize = std::mem::size_of::<libc::c_int>() as _;
                let ret = unsafe {
                    c::getsockopt(
                        fd as _,
                        level,
                        name,
                        &mut flag as *mut libc::c_int as *mut _,
                        &mut flagsize,
                    )
                };
                if ret < 0 {
                    return Err(crate::common::os::errno().into());
                }
                Ok(vm.ctx.new_int(flag).into())
            } else {
                if buflen <= 0 || buflen > 1024 {
                    return Err(vm
                        .new_os_error("getsockopt buflen out of range".to_owned())
                        .into());
                }
                let mut buf = vec![0u8; buflen as usize];
                let mut buflen = buflen as _;
                let ret = unsafe {
                    c::getsockopt(
                        fd as _,
                        level,
                        name,
                        buf.as_mut_ptr() as *mut _,
                        &mut buflen,
                    )
                };
                if ret < 0 {
                    return Err(crate::common::os::errno().into());
                }
                buf.truncate(buflen as usize);
                Ok(vm.ctx.new_bytes(buf).into())
            }
        }

        #[pymethod]
        fn setsockopt(
            &self,
            level: i32,
            name: i32,
            value: Option<Either<ArgBytesLike, i32>>,
            optlen: OptionalArg<u32>,
            vm: &VirtualMachine,
        ) -> Result<(), IoOrPyException> {
            let sock = self.sock()?;
            let fd = sock_fileno(&sock);
            let ret = match (value, optlen) {
                (Some(Either::A(b)), OptionalArg::Missing) => b.with_ref(|b| unsafe {
                    c::setsockopt(fd as _, level, name, b.as_ptr() as *const _, b.len() as _)
                }),
                (Some(Either::B(ref val)), OptionalArg::Missing) => unsafe {
                    c::setsockopt(
                        fd as _,
                        level,
                        name,
                        val as *const i32 as *const _,
                        std::mem::size_of::<i32>() as _,
                    )
                },
                (None, OptionalArg::Present(optlen)) => unsafe {
                    c::setsockopt(fd as _, level, name, std::ptr::null(), optlen as _)
                },
                _ => {
                    return Err(vm
                        .new_type_error("expected the value arg xor the optlen arg".to_owned())
                        .into());
                }
            };
            if ret < 0 {
                Err(crate::common::os::errno().into())
            } else {
                Ok(())
            }
        }

        #[pymethod]
        fn shutdown(&self, how: i32, vm: &VirtualMachine) -> Result<(), IoOrPyException> {
            let how = match how {
                c::SHUT_RD => Shutdown::Read,
                c::SHUT_WR => Shutdown::Write,
                c::SHUT_RDWR => Shutdown::Both,
                _ => {
                    return Err(vm
                        .new_value_error("`how` must be SHUT_RD, SHUT_WR, or SHUT_RDWR".to_owned())
                        .into())
                }
            };
            Ok(self.sock()?.shutdown(how)?)
        }

        #[pygetset(name = "type")]
        fn kind(&self) -> i32 {
            self.kind.load()
        }
        #[pygetset]
        fn family(&self) -> i32 {
            self.family.load()
        }
        #[pygetset]
        fn proto(&self) -> i32 {
            self.proto.load()
        }

        #[pymethod(magic)]
        fn repr(&self) -> String {
            format!(
                "<socket object, fd={}, family={}, type={}, proto={}>",
                // cast because INVALID_SOCKET is unsigned, so would show usize::MAX instead of -1
                self.fileno() as i64,
                self.family.load(),
                self.kind.load(),
                self.proto.load(),
            )
        }
    }

    struct Address {
        host: PyStrRef,
        port: u16,
    }

    impl ToSocketAddrs for Address {
        type Iter = std::vec::IntoIter<SocketAddr>;
        fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
            (self.host.as_str(), self.port).to_socket_addrs()
        }
    }

    impl TryFromObject for Address {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let tuple = PyTupleRef::try_from_object(vm, obj)?;
            if tuple.len() != 2 {
                Err(vm.new_type_error("Address tuple should have only 2 values".to_owned()))
            } else {
                Self::from_tuple(&tuple, vm)
            }
        }
    }

    impl Address {
        fn from_tuple(tuple: &[PyObjectRef], vm: &VirtualMachine) -> PyResult<Self> {
            let host = PyStrRef::try_from_object(vm, tuple[0].clone())?;
            let port = i32::try_from_borrowed_object(vm, &tuple[1])?;
            let port = port
                .to_u16()
                .ok_or_else(|| vm.new_overflow_error("port must be 0-65535.".to_owned()))?;
            Ok(Address { host, port })
        }
        fn from_tuple_ipv6(
            tuple: &[PyObjectRef],
            vm: &VirtualMachine,
        ) -> PyResult<(Self, u32, u32)> {
            let addr = Address::from_tuple(tuple, vm)?;
            let flowinfo = tuple
                .get(2)
                .map(|obj| u32::try_from_borrowed_object(vm, obj))
                .transpose()?
                .unwrap_or(0);
            let scopeid = tuple
                .get(3)
                .map(|obj| u32::try_from_borrowed_object(vm, obj))
                .transpose()?
                .unwrap_or(0);
            if flowinfo > 0xfffff {
                return Err(vm.new_overflow_error("flowinfo must be 0-1048575.".to_owned()));
            }
            Ok((addr, flowinfo, scopeid))
        }
    }

    fn get_ip_addr_tuple(addr: &SocketAddr, vm: &VirtualMachine) -> PyObjectRef {
        match addr {
            SocketAddr::V4(addr) => (addr.ip().to_string(), addr.port()).to_pyobject(vm),
            SocketAddr::V6(addr) => (
                addr.ip().to_string(),
                addr.port(),
                addr.flowinfo(),
                addr.scope_id(),
            )
                .to_pyobject(vm),
        }
    }

    fn get_addr_tuple(addr: &socket2::SockAddr, vm: &VirtualMachine) -> PyObjectRef {
        if let Some(addr) = addr.as_socket() {
            return get_ip_addr_tuple(&addr, vm);
        }
        match addr.family() as i32 {
            #[cfg(unix)]
            libc::AF_UNIX => {
                let addr_len = addr.len() as usize;
                let unix_addr = unsafe { &*(addr.as_ptr() as *const libc::sockaddr_un) };
                let path_u8 = unsafe { &*(&unix_addr.sun_path[..] as *const [_] as *const [u8]) };
                let sun_path_offset =
                    &unix_addr.sun_path as *const _ as usize - unix_addr as *const _ as usize;
                if cfg!(any(target_os = "linux", target_os = "android"))
                    && addr_len > sun_path_offset
                    && unix_addr.sun_path[0] == 0
                {
                    let abstractaddrlen = addr_len - sun_path_offset;
                    let abstractpath = &path_u8[..abstractaddrlen];
                    vm.ctx.new_bytes(abstractpath.to_vec()).into()
                } else {
                    let len = memchr::memchr(b'\0', path_u8).unwrap_or(path_u8.len());
                    let path = &path_u8[..len];
                    vm.ctx.new_str(String::from_utf8_lossy(path)).into()
                }
            }
            // TODO: support more address families
            _ => (String::new(), 0).to_pyobject(vm),
        }
    }

    #[pyfunction]
    fn gethostname(vm: &VirtualMachine) -> PyResult<PyStrRef> {
        gethostname::gethostname()
            .into_string()
            .map(|hostname| vm.ctx.new_str(hostname))
            .map_err(|err| vm.new_os_error(err.into_string().unwrap()))
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    #[pyfunction]
    fn sethostname(hostname: PyStrRef) -> nix::Result<()> {
        nix::unistd::sethostname(hostname.as_str())
    }

    #[pyfunction]
    fn inet_aton(ip_string: PyStrRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        ip_string
            .as_str()
            .parse::<Ipv4Addr>()
            .map(|ip_addr| Vec::<u8>::from(ip_addr.octets()))
            .map_err(|_| {
                vm.new_os_error("illegal IP address string passed to inet_aton".to_owned())
            })
    }

    #[pyfunction]
    fn inet_ntoa(packed_ip: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let packed_ip = packed_ip.borrow_buf();
        let packed_ip = <&[u8; 4]>::try_from(&*packed_ip)
            .map_err(|_| vm.new_os_error("packed IP wrong length for inet_ntoa".to_owned()))?;
        Ok(vm.ctx.new_str(Ipv4Addr::from(*packed_ip).to_string()))
    }

    fn cstr_opt_as_ptr(x: &OptionalArg<ffi::CString>) -> *const libc::c_char {
        x.as_ref().map_or_else(std::ptr::null, |s| s.as_ptr())
    }

    #[pyfunction]
    fn getservbyname(
        servicename: PyStrRef,
        protocolname: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<u16> {
        let cstr_name = servicename.to_cstring(vm)?;
        let cstr_proto = protocolname
            .as_ref()
            .map(|s| s.to_cstring(vm))
            .transpose()?;
        let cstr_proto = cstr_opt_as_ptr(&cstr_proto);
        let serv = unsafe { c::getservbyname(cstr_name.as_ptr(), cstr_proto) };
        if serv.is_null() {
            return Err(vm.new_os_error("service/proto not found".to_owned()));
        }
        let port = unsafe { (*serv).s_port };
        Ok(u16::from_be(port as u16))
    }

    #[pyfunction]
    fn getservbyport(
        port: i32,
        protocolname: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let port = port.to_u16().ok_or_else(|| {
            vm.new_overflow_error("getservbyport: port must be 0-65535.".to_owned())
        })?;
        let cstr_proto = protocolname
            .as_ref()
            .map(|s| s.to_cstring(vm))
            .transpose()?;
        let cstr_proto = cstr_opt_as_ptr(&cstr_proto);
        let serv = unsafe { c::getservbyport(port.to_be() as _, cstr_proto) };
        if serv.is_null() {
            return Err(vm.new_os_error("port/proto not found".to_owned()));
        }
        let s = unsafe { ffi::CStr::from_ptr((*serv).s_name) };
        Ok(s.to_string_lossy().into_owned())
    }

    fn slice_as_uninit<T>(v: &mut [T]) -> &mut [MaybeUninit<T>] {
        unsafe { &mut *(v as *mut [T] as *mut [MaybeUninit<T>]) }
    }

    enum IoOrPyException {
        Timeout,
        Py(PyBaseExceptionRef),
        Io(io::Error),
    }
    impl From<PyBaseExceptionRef> for IoOrPyException {
        fn from(exc: PyBaseExceptionRef) -> Self {
            Self::Py(exc)
        }
    }
    impl From<io::Error> for IoOrPyException {
        fn from(err: io::Error) -> Self {
            Self::Io(err)
        }
    }
    impl IoOrPyException {
        fn errno(self) -> PyResult<i32> {
            match self {
                Self::Timeout => Ok(errcode!(EWOULDBLOCK)),
                Self::Io(err) => {
                    // TODO: just unwrap()?
                    Ok(err.raw_os_error().unwrap_or(1))
                }
                Self::Py(exc) => Err(exc),
            }
        }
    }
    impl IntoPyException for IoOrPyException {
        #[inline]
        fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            match self {
                Self::Timeout => timeout_error(vm),
                Self::Py(exc) => exc,
                Self::Io(err) => err.into_pyexception(vm),
            }
        }
    }

    #[derive(Copy, Clone)]
    pub(crate) enum SelectKind {
        Read,
        Write,
        Connect,
    }

    /// returns true if timed out
    pub(crate) fn sock_select(
        sock: &Socket,
        kind: SelectKind,
        interval: Option<Duration>,
    ) -> io::Result<bool> {
        let fd = sock_fileno(sock);
        #[cfg(unix)]
        {
            let mut pollfd = libc::pollfd {
                fd,
                events: match kind {
                    SelectKind::Read => libc::POLLIN,
                    SelectKind::Write => libc::POLLOUT,
                    SelectKind::Connect => libc::POLLOUT | libc::POLLERR,
                },
                revents: 0,
            };
            let timeout = match interval {
                Some(d) => d.as_millis() as _,
                None => -1,
            };
            let ret = unsafe { libc::poll(&mut pollfd, 1, timeout) };
            if ret < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(ret == 0)
            }
        }
        #[cfg(windows)]
        {
            use crate::select;

            let mut reads = select::FdSet::new();
            let mut writes = select::FdSet::new();
            let mut errs = select::FdSet::new();

            let fd = fd as usize;
            match kind {
                SelectKind::Read => reads.insert(fd),
                SelectKind::Write => writes.insert(fd),
                SelectKind::Connect => {
                    writes.insert(fd);
                    errs.insert(fd);
                }
            }

            let mut interval = interval.map(|dur| select::timeval {
                tv_sec: dur.as_secs() as _,
                tv_usec: dur.subsec_micros() as _,
            });

            select::select(
                fd as i32 + 1,
                &mut reads,
                &mut writes,
                &mut errs,
                interval.as_mut(),
            )
            .map(|ret| ret == 0)
        }
    }

    #[derive(FromArgs)]
    struct GAIOptions {
        #[pyarg(positional)]
        host: Option<PyStrRef>,
        #[pyarg(positional)]
        port: Option<Either<PyStrRef, i32>>,

        #[pyarg(positional, default = "c::AF_UNSPEC")]
        family: i32,
        #[pyarg(positional, default = "0")]
        ty: i32,
        #[pyarg(positional, default = "0")]
        proto: i32,
        #[pyarg(positional, default = "0")]
        flags: i32,
    }

    #[pyfunction]
    fn getaddrinfo(
        opts: GAIOptions,
        vm: &VirtualMachine,
    ) -> Result<Vec<PyObjectRef>, IoOrPyException> {
        let hints = dns_lookup::AddrInfoHints {
            socktype: opts.ty,
            protocol: opts.proto,
            address: opts.family,
            flags: opts.flags,
        };

        let host = opts.host.as_ref().map(|s| s.as_str());
        let port = opts.port.as_ref().map(|p| -> std::borrow::Cow<str> {
            match p {
                Either::A(ref s) => s.as_str().into(),
                Either::B(i) => i.to_string().into(),
            }
        });
        let port = port.as_ref().map(|p| p.as_ref());

        let addrs = dns_lookup::getaddrinfo(host, port, Some(hints))
            .map_err(|err| convert_socket_error(vm, err, SocketError::GaiError))?;

        let list = addrs
            .map(|ai| {
                ai.map(|ai| {
                    vm.new_tuple((
                        ai.address,
                        ai.socktype,
                        ai.protocol,
                        ai.canonname,
                        get_ip_addr_tuple(&ai.sockaddr, vm),
                    ))
                    .into()
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(list)
    }

    #[pyfunction]
    fn gethostbyaddr(
        addr: PyStrRef,
        vm: &VirtualMachine,
    ) -> Result<(String, PyListRef, PyListRef), IoOrPyException> {
        let addr = get_addr(vm, addr, c::AF_UNSPEC)?;
        let (hostname, _) = dns_lookup::getnameinfo(&addr, 0)
            .map_err(|e| convert_socket_error(vm, e, SocketError::HError))?;
        Ok((
            hostname,
            vm.ctx.new_list(vec![]),
            vm.ctx
                .new_list(vec![vm.ctx.new_str(addr.ip().to_string()).into()]),
        ))
    }

    #[pyfunction]
    fn gethostbyname(name: PyStrRef, vm: &VirtualMachine) -> Result<String, IoOrPyException> {
        let addr = get_addr(vm, name, c::AF_INET)?;
        match addr {
            SocketAddr::V4(ip) => Ok(ip.ip().to_string()),
            _ => unreachable!(),
        }
    }

    #[pyfunction]
    fn gethostbyname_ex(
        name: PyStrRef,
        vm: &VirtualMachine,
    ) -> Result<(String, PyListRef, PyListRef), IoOrPyException> {
        let addr = get_addr(vm, name, c::AF_UNSPEC)?;
        let (hostname, _) = dns_lookup::getnameinfo(&addr, 0)
            .map_err(|e| convert_socket_error(vm, e, SocketError::HError))?;
        Ok((
            hostname,
            vm.ctx.new_list(vec![]),
            vm.ctx
                .new_list(vec![vm.ctx.new_str(addr.ip().to_string()).into()]),
        ))
    }

    #[pyfunction]
    fn inet_pton(af_inet: i32, ip_string: PyStrRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        static ERROR_MSG: &str = "illegal IP address string passed to inet_pton";
        let ip_addr = match af_inet {
            c::AF_INET => ip_string
                .as_str()
                .parse::<Ipv4Addr>()
                .map_err(|_| vm.new_os_error(ERROR_MSG.to_owned()))?
                .octets()
                .to_vec(),
            c::AF_INET6 => ip_string
                .as_str()
                .parse::<Ipv6Addr>()
                .map_err(|_| vm.new_os_error(ERROR_MSG.to_owned()))?
                .octets()
                .to_vec(),
            _ => return Err(vm.new_os_error("Address family not supported by protocol".to_owned())),
        };
        Ok(ip_addr)
    }

    #[pyfunction]
    fn inet_ntop(af_inet: i32, packed_ip: ArgBytesLike, vm: &VirtualMachine) -> PyResult<String> {
        let packed_ip = packed_ip.borrow_buf();
        match af_inet {
            c::AF_INET => {
                let packed_ip = <&[u8; 4]>::try_from(&*packed_ip).map_err(|_| {
                    vm.new_value_error("invalid length of packed IP address string".to_owned())
                })?;
                Ok(Ipv4Addr::from(*packed_ip).to_string())
            }
            c::AF_INET6 => {
                let packed_ip = <&[u8; 16]>::try_from(&*packed_ip).map_err(|_| {
                    vm.new_value_error("invalid length of packed IP address string".to_owned())
                })?;
                Ok(get_ipv6_addr_str(Ipv6Addr::from(*packed_ip)))
            }
            _ => Err(vm.new_value_error(format!("unknown address family {}", af_inet))),
        }
    }

    #[pyfunction]
    fn getprotobyname(name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let cstr = name.to_cstring(vm)?;
        let proto = unsafe { c::getprotobyname(cstr.as_ptr()) };
        if proto.is_null() {
            return Err(vm.new_os_error("protocol not found".to_owned()));
        }
        let num = unsafe { (*proto).p_proto };
        Ok(vm.ctx.new_int(num).into())
    }

    #[pyfunction]
    fn getnameinfo(
        address: PyTupleRef,
        flags: i32,
        vm: &VirtualMachine,
    ) -> Result<(String, String), IoOrPyException> {
        match address.len() {
            2 | 3 | 4 => {}
            _ => {
                return Err(vm
                    .new_type_error("illegal sockaddr argument".to_owned())
                    .into())
            }
        }
        let (addr, flowinfo, scopeid) = Address::from_tuple_ipv6(&address, vm)?;
        let hints = dns_lookup::AddrInfoHints {
            address: c::AF_UNSPEC,
            socktype: c::SOCK_DGRAM,
            flags: c::AI_NUMERICHOST,
            protocol: 0,
        };
        let service = addr.port.to_string();
        let mut res =
            dns_lookup::getaddrinfo(Some(addr.host.as_str()), Some(&service), Some(hints))
                .map_err(|e| convert_socket_error(vm, e, SocketError::GaiError))?
                .filter_map(Result::ok);
        let mut ainfo = res.next().unwrap();
        if res.next().is_some() {
            return Err(vm
                .new_os_error("sockaddr resolved to multiple addresses".to_owned())
                .into());
        }
        match &mut ainfo.sockaddr {
            SocketAddr::V4(_) => {
                if address.len() != 2 {
                    return Err(vm
                        .new_os_error("IPv4 sockaddr must be 2 tuple".to_owned())
                        .into());
                }
            }
            SocketAddr::V6(addr) => {
                addr.set_flowinfo(flowinfo);
                addr.set_scope_id(scopeid);
            }
        }
        dns_lookup::getnameinfo(&ainfo.sockaddr, flags)
            .map_err(|e| convert_socket_error(vm, e, SocketError::GaiError))
    }

    #[cfg(unix)]
    #[pyfunction]
    fn socketpair(
        family: OptionalArg<i32>,
        socket_kind: OptionalArg<i32>,
        proto: OptionalArg<i32>,
    ) -> Result<(PySocket, PySocket), IoOrPyException> {
        let family = family.unwrap_or(libc::AF_UNIX);
        let socket_kind = socket_kind.unwrap_or(libc::SOCK_STREAM);
        let proto = proto.unwrap_or(0);
        let (a, b) = Socket::pair(family.into(), socket_kind.into(), Some(proto.into()))?;
        let py_a = PySocket::default();
        py_a.init_inner(family, socket_kind, proto, a)?;
        let py_b = PySocket::default();
        py_b.init_inner(family, socket_kind, proto, b)?;
        Ok((py_a, py_b))
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    type IfIndex = c::c_uint;
    #[cfg(windows)]
    type IfIndex = winapi::shared::ifdef::NET_IFINDEX;

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn if_nametoindex(name: PyObjectRef, vm: &VirtualMachine) -> PyResult<IfIndex> {
        let name = crate::vm::stdlib::os::FsPath::try_from(name, true, vm)?;
        let name = ffi::CString::new(name.as_bytes()).map_err(|err| err.into_pyexception(vm))?;

        let ret = unsafe { c::if_nametoindex(name.as_ptr()) };

        if ret == 0 {
            Err(vm.new_os_error("no interface with this name".to_owned()))
        } else {
            Ok(ret)
        }
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn if_indextoname(index: IfIndex, vm: &VirtualMachine) -> PyResult<String> {
        let mut buf = [0; c::IF_NAMESIZE + 1];
        let ret = unsafe { c::if_indextoname(index, buf.as_mut_ptr()) };
        if ret.is_null() {
            Err(crate::vm::stdlib::os::errno_err(vm))
        } else {
            let buf = unsafe { ffi::CStr::from_ptr(buf.as_ptr()) };
            Ok(buf.to_string_lossy().into_owned())
        }
    }

    #[cfg(any(
        windows,
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "ios",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    #[pyfunction]
    fn if_nameindex(vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        #[cfg(not(windows))]
        {
            let list = nix::net::if_::if_nameindex()
                .map_err(|err| err.into_pyexception(vm))?
                .to_slice()
                .iter()
                .map(|iface| {
                    let tup: (u32, String) =
                        (iface.index(), iface.name().to_string_lossy().into_owned());
                    tup.to_pyobject(vm)
                })
                .collect();

            Ok(list)
        }
        #[cfg(windows)]
        {
            use std::ptr;

            let table = MibTable::get_raw().map_err(|err| err.into_pyexception(vm))?;
            let list = table.as_slice().iter().map(|entry| {
                let name =
                    get_name(&entry.InterfaceLuid).map_err(|err| err.into_pyexception(vm))?;
                let tup = (entry.InterfaceIndex, name.to_string_lossy());
                Ok(tup.to_pyobject(vm))
            });
            let list = list.collect::<PyResult<_>>()?;
            return Ok(list);

            fn get_name(
                luid: &winapi::shared::ifdef::NET_LUID,
            ) -> io::Result<widestring::WideCString> {
                let mut buf = [0; c::IF_NAMESIZE + 1];
                let ret = unsafe {
                    netioapi::ConvertInterfaceLuidToNameW(luid, buf.as_mut_ptr(), buf.len())
                };
                if ret == 0 {
                    Ok(widestring::WideCString::from_ustr_truncate(
                        widestring::WideStr::from_slice(&buf[..]),
                    ))
                } else {
                    Err(io::Error::from_raw_os_error(ret as i32))
                }
            }
            struct MibTable {
                ptr: ptr::NonNull<netioapi::MIB_IF_TABLE2>,
            }
            impl MibTable {
                fn get_raw() -> io::Result<Self> {
                    let mut ptr = ptr::null_mut();
                    let ret = unsafe { netioapi::GetIfTable2Ex(netioapi::MibIfTableRaw, &mut ptr) };
                    if ret == 0 {
                        let ptr = unsafe { ptr::NonNull::new_unchecked(ptr) };
                        Ok(Self { ptr })
                    } else {
                        Err(io::Error::from_raw_os_error(ret as i32))
                    }
                }
            }
            impl MibTable {
                fn as_slice(&self) -> &[netioapi::MIB_IF_ROW2] {
                    unsafe {
                        let p = self.ptr.as_ptr();
                        let ptr = ptr::addr_of!((*p).Table) as *const netioapi::MIB_IF_ROW2;
                        std::slice::from_raw_parts(ptr, (*p).NumEntries as usize)
                    }
                }
            }
            impl Drop for MibTable {
                fn drop(&mut self) {
                    unsafe { netioapi::FreeMibTable(self.ptr.as_ptr() as *mut _) }
                }
            }
        }
    }

    fn get_addr(
        vm: &VirtualMachine,
        pyname: PyStrRef,
        af: i32,
    ) -> Result<SocketAddr, IoOrPyException> {
        let name = pyname.as_str();
        if name.is_empty() {
            let hints = dns_lookup::AddrInfoHints {
                address: af,
                socktype: c::SOCK_DGRAM,
                flags: c::AI_PASSIVE,
                protocol: 0,
            };
            let mut res = dns_lookup::getaddrinfo(None, Some("0"), Some(hints))
                .map_err(|e| convert_socket_error(vm, e, SocketError::GaiError))?;
            let ainfo = res.next().unwrap()?;
            if res.next().is_some() {
                return Err(vm
                    .new_os_error("wildcard resolved to multiple address".to_owned())
                    .into());
            }
            return Ok(ainfo.sockaddr);
        }
        if name == "255.255.255.255" || name == "<broadcast>" {
            match af {
                c::AF_INET | c::AF_UNSPEC => {}
                _ => {
                    return Err(vm
                        .new_os_error("address family mismatched".to_owned())
                        .into())
                }
            }
            return Ok(SocketAddr::V4(net::SocketAddrV4::new(
                c::INADDR_BROADCAST.into(),
                0,
            )));
        }
        if let c::AF_INET | c::AF_UNSPEC = af {
            if let Ok(addr) = name.parse::<Ipv4Addr>() {
                return Ok(SocketAddr::V4(net::SocketAddrV4::new(addr, 0)));
            }
        }
        if matches!(af, c::AF_INET | c::AF_UNSPEC) && !name.contains('%') {
            if let Ok(addr) = name.parse::<Ipv6Addr>() {
                return Ok(SocketAddr::V6(net::SocketAddrV6::new(addr, 0, 0, 0)));
            }
        }
        let hints = dns_lookup::AddrInfoHints {
            address: af,
            ..Default::default()
        };
        let name = vm
            .state
            .codec_registry
            .encode_text(pyname, "idna", None, vm)?;
        let name = std::str::from_utf8(name.as_bytes())
            .map_err(|_| vm.new_runtime_error("idna output is not utf8".to_owned()))?;
        let mut res = dns_lookup::getaddrinfo(Some(name), None, Some(hints))
            .map_err(|e| convert_socket_error(vm, e, SocketError::GaiError))?;
        Ok(res.next().unwrap().map(|ainfo| ainfo.sockaddr)?)
    }

    fn sock_from_raw(fileno: RawSocket, vm: &VirtualMachine) -> PyResult<Socket> {
        let invalid = {
            cfg_if::cfg_if! {
                if #[cfg(windows)] {
                    fileno == INVALID_SOCKET
                } else {
                    fileno < 0
                }
            }
        };
        if invalid {
            return Err(vm.new_value_error("negative file descriptor".to_owned()));
        }
        Ok(unsafe { sock_from_raw_unchecked(fileno) })
    }
    /// SAFETY: fileno must not be equal to INVALID_SOCKET
    unsafe fn sock_from_raw_unchecked(fileno: RawSocket) -> Socket {
        #[cfg(unix)]
        {
            use std::os::unix::io::FromRawFd;
            Socket::from_raw_fd(fileno)
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::FromRawSocket;
            Socket::from_raw_socket(fileno)
        }
    }
    pub(super) fn sock_fileno(sock: &Socket) -> RawSocket {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            sock.as_raw_fd()
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawSocket;
            sock.as_raw_socket()
        }
    }
    fn into_sock_fileno(sock: Socket) -> RawSocket {
        #[cfg(unix)]
        {
            use std::os::unix::io::IntoRawFd;
            sock.into_raw_fd()
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::IntoRawSocket;
            sock.into_raw_socket()
        }
    }

    pub(super) const INVALID_SOCKET: RawSocket = {
        #[cfg(unix)]
        {
            -1
        }
        #[cfg(windows)]
        {
            winapi::um::winsock2::INVALID_SOCKET as RawSocket
        }
    };

    fn convert_socket_error(
        vm: &VirtualMachine,
        err: dns_lookup::LookupError,
        err_kind: SocketError,
    ) -> IoOrPyException {
        if let dns_lookup::LookupErrorKind::System = err.kind() {
            return io::Error::from(err).into();
        }
        let strerr = {
            #[cfg(unix)]
            {
                let s = match err_kind {
                    SocketError::GaiError => unsafe {
                        ffi::CStr::from_ptr(libc::gai_strerror(err.error_num()))
                    },
                    SocketError::HError => unsafe {
                        ffi::CStr::from_ptr(libc::hstrerror(err.error_num()))
                    },
                };
                s.to_str().unwrap()
            }
            #[cfg(windows)]
            {
                "getaddrinfo failed"
            }
        };
        let exception_cls = match err_kind {
            SocketError::GaiError => gaierror(vm),
            SocketError::HError => herror(vm),
        };
        vm.new_exception(
            exception_cls,
            vec![vm.new_pyobj(err.error_num()), vm.ctx.new_str(strerr).into()],
        )
        .into()
    }

    fn timeout_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
        timeout_error_msg(vm, "timed out".to_owned())
    }
    pub(crate) fn timeout_error_msg(vm: &VirtualMachine, msg: String) -> PyBaseExceptionRef {
        vm.new_exception_msg(timeout(vm), msg)
    }

    fn get_ipv6_addr_str(ipv6: Ipv6Addr) -> String {
        match ipv6.to_ipv4() {
            // instead of "::0.0.ddd.ddd" it's "::xxxx"
            Some(v4) if !ipv6.is_unspecified() && matches!(v4.octets(), [0, 0, _, _]) => {
                format!("::{:x}", u32::from(v4))
            }
            _ => ipv6.to_string(),
        }
    }

    pub(crate) struct Deadline {
        deadline: Instant,
    }

    impl Deadline {
        fn new(timeout: Duration) -> Self {
            Self {
                deadline: Instant::now() + timeout,
            }
        }
        fn time_until(&self) -> Result<Duration, IoOrPyException> {
            self.deadline
                .checked_duration_since(Instant::now())
                // past the deadline already
                .ok_or(IoOrPyException::Timeout)
        }
    }

    static DEFAULT_TIMEOUT: AtomicCell<f64> = AtomicCell::new(-1.0);

    #[pyfunction]
    fn getdefaulttimeout() -> Option<f64> {
        let timeout = DEFAULT_TIMEOUT.load();
        if timeout >= 0.0 {
            Some(timeout)
        } else {
            None
        }
    }

    #[pyfunction]
    fn setdefaulttimeout(timeout: Option<Duration>) {
        DEFAULT_TIMEOUT.store(timeout.map_or(-1.0, |d| d.as_secs_f64()));
    }

    #[pyfunction]
    fn dup(x: PyObjectRef, vm: &VirtualMachine) -> Result<RawSocket, IoOrPyException> {
        let sock = get_raw_sock(x, vm)?;
        let sock = std::mem::ManuallyDrop::new(sock_from_raw(sock, vm)?);
        let newsock = sock.try_clone()?;
        let fd = into_sock_fileno(newsock);
        #[cfg(windows)]
        crate::vm::stdlib::nt::raw_set_handle_inheritable(fd as _, false)?;
        Ok(fd)
    }

    #[pyfunction]
    fn close(x: PyObjectRef, vm: &VirtualMachine) -> Result<(), IoOrPyException> {
        Ok(close_inner(get_raw_sock(x, vm)?)?)
    }

    fn close_inner(x: RawSocket) -> io::Result<()> {
        #[cfg(unix)]
        use libc::close;
        #[cfg(windows)]
        use winapi::um::winsock2::closesocket as close;
        let ret = unsafe { close(x as _) };
        if ret < 0 {
            let err = crate::common::os::errno();
            if err.raw_os_error() != Some(errcode!(ECONNRESET)) {
                return Err(err);
            }
        }
        Ok(())
    }

    enum SocketError {
        HError,
        GaiError,
    }
}
