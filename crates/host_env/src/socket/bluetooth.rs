//! Linux Bluetooth socket address layouts from the BlueZ headers.

use super::raw::{SockAddr, SockAddrStorage};

pub const L2CAP: i32 = 0;
pub const HCI: i32 = 1;
pub const SCO: i32 = 2;
pub const RFCOMM: i32 = 3;

#[repr(C)]
struct SockaddrL2 {
    family: libc::sa_family_t,
    psm: u16,
    bdaddr: [u8; 6],
    cid: u16,
    addr_type: u8,
}

#[repr(C)]
struct SockaddrRc {
    family: libc::sa_family_t,
    bdaddr: [u8; 6],
    channel: u8,
}

#[repr(C)]
struct SockaddrHci {
    family: libc::sa_family_t,
    dev: u16,
    channel: u16,
}

#[repr(C)]
struct SockaddrSco {
    family: libc::sa_family_t,
    bdaddr: [u8; 6],
}

#[derive(Debug, PartialEq, Eq)]
pub enum Address {
    L2cap {
        bdaddr: [u8; 6],
        psm: u16,
        cid: u16,
        addr_type: u8,
    },
    Rfcomm {
        bdaddr: [u8; 6],
        channel: u8,
    },
    Hci {
        dev: u16,
        channel: u16,
    },
    Sco {
        bdaddr: [u8; 6],
    },
}

impl Address {
    pub fn to_sockaddr(&self) -> SockAddr {
        let mut storage = SockAddrStorage::zeroed();
        // SAFETY: these repr(C) layouts match the Linux sockaddr_* definitions.
        // SockAddrStorage provides their size/alignment and is zero-initialized.
        // Write fields in place so padding remains initialized.
        let len = unsafe {
            match *self {
                Self::L2cap {
                    bdaddr,
                    psm,
                    cid,
                    addr_type,
                } => {
                    let addr = storage.view_as::<SockaddrL2>();
                    addr.family = libc::AF_BLUETOOTH as _;
                    addr.psm = psm;
                    addr.bdaddr = bdaddr;
                    addr.cid = cid;
                    addr.addr_type = addr_type;
                    size_of::<SockaddrL2>()
                }
                Self::Rfcomm { bdaddr, channel } => {
                    let addr = storage.view_as::<SockaddrRc>();
                    addr.family = libc::AF_BLUETOOTH as _;
                    addr.bdaddr = bdaddr;
                    addr.channel = channel;
                    size_of::<SockaddrRc>()
                }
                Self::Hci { dev, channel } => {
                    let addr = storage.view_as::<SockaddrHci>();
                    addr.family = libc::AF_BLUETOOTH as _;
                    addr.dev = dev;
                    addr.channel = channel;
                    size_of::<SockaddrHci>()
                }
                Self::Sco { bdaddr } => {
                    let addr = storage.view_as::<SockaddrSco>();
                    addr.family = libc::AF_BLUETOOTH as _;
                    addr.bdaddr = bdaddr;
                    size_of::<SockaddrSco>()
                }
            }
        };
        // SAFETY: storage contains a complete address of the stated length.
        unsafe { SockAddr::new(storage, len as _) }
    }

    pub fn from_sockaddr(addr: &SockAddr, protocol: i32) -> Option<Self> {
        if addr.family() != libc::AF_BLUETOOTH as _ {
            return None;
        }
        // SAFETY: SockAddr owns aligned storage; each cast checks the returned
        // length. All bit patterns are valid for these integer-only layouts.
        unsafe {
            match protocol {
                L2CAP if addr.len() as usize >= size_of::<SockaddrL2>() => {
                    let addr = &*addr.as_ptr().cast::<SockaddrL2>();
                    Some(Self::L2cap {
                        bdaddr: addr.bdaddr,
                        psm: addr.psm,
                        cid: addr.cid,
                        addr_type: addr.addr_type,
                    })
                }
                RFCOMM if addr.len() as usize >= size_of::<SockaddrRc>() => {
                    let addr = &*addr.as_ptr().cast::<SockaddrRc>();
                    Some(Self::Rfcomm {
                        bdaddr: addr.bdaddr,
                        channel: addr.channel,
                    })
                }
                HCI if addr.len() as usize >= size_of::<SockaddrHci>() => {
                    let addr = &*addr.as_ptr().cast::<SockaddrHci>();
                    Some(Self::Hci {
                        dev: addr.dev,
                        channel: addr.channel,
                    })
                }
                SCO if addr.len() as usize >= size_of::<SockaddrSco>() => {
                    let addr = &*addr.as_ptr().cast::<SockaddrSco>();
                    Some(Self::Sco {
                        bdaddr: addr.bdaddr,
                    })
                }
                _ => None,
            }
        }
    }
}

pub fn parse_bdaddr(name: &[u8]) -> Option<[u8; 6]> {
    let mut parts = name.split(|&b| b == b':');
    let mut bdaddr = [0; 6];
    for byte in bdaddr.iter_mut().rev() {
        // CPython's %X parser permits leading whitespace, a sign and 0x.
        let part = parts.next()?.trim_ascii_start();
        let (part, negative) = match part.first() {
            Some(b'+') => (&part[1..], false),
            Some(b'-') => (&part[1..], true),
            _ => (part, false),
        };
        if part.is_empty() {
            return None;
        }
        let part = part
            .strip_prefix(b"0x")
            .or_else(|| part.strip_prefix(b"0X"))
            .unwrap_or(part);
        let mut value = 0u8;
        for &digit in part {
            let digit = (digit as char).to_digit(16)? as u8;
            value = value.checked_mul(16)?.checked_add(digit)?;
        }
        if negative && value != 0 {
            return None;
        }
        *byte = value;
    }
    parts.next().is_none().then_some(bdaddr)
}

pub fn format_bdaddr(bdaddr: [u8; 6]) -> String {
    let [f, e, d, c, b, a] = bdaddr;
    alloc::format!("{a:02X}:{b:02X}:{c:02X}:{d:02X}:{e:02X}:{f:02X}")
}
