pub fn tcgetattr(fd: i32) -> std::io::Result<::termios::Termios> {
    ::termios::Termios::from_fd(fd)
}

pub fn tcsetattr(fd: i32, when: i32, termios: &::termios::Termios) -> std::io::Result<()> {
    ::termios::tcsetattr(fd, when, termios)
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
