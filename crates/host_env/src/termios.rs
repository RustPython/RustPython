pub type Termios = ::termios::Termios;

#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "illumos",
    target_os = "linux",
    target_os = "macos",
    target_os = "openbsd",
    target_os = "solaris"
))]
pub use ::termios::os::target::TAB3;
#[cfg(any(
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "macos",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub use ::termios::os::target::TCSASOFT;
#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "illumos",
    target_os = "linux",
    target_os = "netbsd",
    target_os = "solaris"
))]
pub use ::termios::os::target::{B460800, B921600};
#[cfg(any(target_os = "android", target_os = "linux"))]
pub use ::termios::os::target::{
    B500000, B576000, B1000000, B1152000, B1500000, B2000000, B2500000, B3000000, B3500000,
    B4000000, CBAUDEX,
};
#[cfg(any(
    target_os = "android",
    target_os = "illumos",
    target_os = "linux",
    target_os = "macos",
    target_os = "solaris"
))]
pub use ::termios::os::target::{
    BS0, BS1, BSDLY, CR0, CR1, CR2, CR3, CRDLY, FF0, FF1, FFDLY, NL0, NL1, NLDLY, OFDEL, OFILL,
    TAB1, TAB2, VT0, VT1, VTDLY,
};
#[cfg(any(
    target_os = "android",
    target_os = "illumos",
    target_os = "linux",
    target_os = "solaris"
))]
pub use ::termios::os::target::{CBAUD, CIBAUD, IUCLC, OLCUC, XCASE};
#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "illumos",
    target_os = "linux",
    target_os = "macos",
    target_os = "solaris"
))]
pub use ::termios::os::target::{TAB0, TABDLY};
#[cfg(any(target_os = "android", target_os = "linux"))]
pub use ::termios::os::target::{VSWTC, VSWTC as VSWTCH};
#[cfg(any(target_os = "illumos", target_os = "solaris"))]
pub use ::termios::os::target::{VSWTCH, VSWTCH as VSWTC};
pub use ::termios::{
    B0, B50, B75, B110, B134, B150, B200, B300, B600, B1200, B1800, B2400, B4800, B9600, B19200,
    B38400, BRKINT, CLOCAL, CREAD, CS5, CS6, CS7, CS8, CSIZE, CSTOPB, ECHO, ECHOE, ECHOK, ECHONL,
    HUPCL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, IGNPAR, INLCR, INPCK, ISIG, ISTRIP, IXANY, IXOFF,
    IXON, NOFLSH, OCRNL, ONLCR, ONLRET, ONOCR, OPOST, PARENB, PARMRK, PARODD, TCIFLUSH, TCIOFF,
    TCIOFLUSH, TCION, TCOFLUSH, TCOOFF, TCOON, TCSADRAIN, TCSAFLUSH, TCSANOW, TOSTOP, VEOF, VEOL,
    VERASE, VINTR, VKILL, VMIN, VQUIT, VSTART, VSTOP, VSUSP, VTIME,
    os::target::{
        B57600, B115200, B230400, CRTSCTS, ECHOCTL, ECHOKE, ECHOPRT, EXTA, EXTB, FLUSHO, IMAXBEL,
        NCCS, PENDIN, VDISCARD, VEOL2, VLNEXT, VREPRINT, VWERASE,
    },
};

pub fn tcgetattr(fd: i32) -> std::io::Result<Termios> {
    Termios::from_fd(fd)
}

pub fn tcsetattr(fd: i32, when: i32, termios: &Termios) -> std::io::Result<()> {
    ::termios::tcsetattr(fd, when, termios)
}

pub fn cfgetispeed(termios: &Termios) -> libc::speed_t {
    ::termios::cfgetispeed(termios)
}

pub fn cfgetospeed(termios: &Termios) -> libc::speed_t {
    ::termios::cfgetospeed(termios)
}

pub fn cfsetispeed(termios: &mut Termios, speed: libc::speed_t) -> std::io::Result<()> {
    ::termios::cfsetispeed(termios, speed)
}

pub fn cfsetospeed(termios: &mut Termios, speed: libc::speed_t) -> std::io::Result<()> {
    ::termios::cfsetospeed(termios, speed)
}

pub fn tcsendbreak(fd: i32, duration: i32) -> std::io::Result<()> {
    ::termios::tcsendbreak(fd, duration)
}

pub fn tcdrain(fd: i32) -> std::io::Result<()> {
    ::termios::tcdrain(fd)
}

pub fn tcflush(fd: i32, queue: i32) -> std::io::Result<()> {
    ::termios::tcflush(fd, queue)
}

pub fn tcflow(fd: i32, action: i32) -> std::io::Result<()> {
    ::termios::tcflow(fd, action)
}
