use std::io;

pub(super) const COOKIE: &[u8; 8] = b"xdebugpy";

pub(super) trait RemoteRead {
    fn read_exact(&mut self, address: u64, destination: &mut [u8]) -> io::Result<()>;
}

pub(super) fn has_cookie(reader: &mut impl RemoteRead, address: u64) -> io::Result<bool> {
    if address == 0 {
        return Ok(false);
    }
    let mut bytes = [0; COOKIE.len()];
    match reader.read_exact(address, &mut bytes) {
        Ok(()) => Ok(&bytes == COOKIE),
        Err(error) if stops_probe(&error) => Err(error),
        Err(_) => Ok(false),
    }
}

pub(super) fn stops_probe(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::OutOfMemory
    )
}

pub(super) fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

// These are resource limits, not claims about an unwinder's capabilities.
// Exhaustion is a failed probe, just like inaccessible or missing metadata.
pub(super) struct ReadBudget {
    bytes: usize,
    calls: usize,
}

impl ReadBudget {
    pub(super) fn new() -> Self {
        Self {
            bytes: 256 * 1024 * 1024,
            calls: 65_536,
        }
    }

    pub(super) fn consume(&mut self, bytes: usize) -> io::Result<()> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::OutOfMemory,
                "runtime probe byte budget exhausted",
            )
        })?;
        self.calls = self.calls.checked_sub(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::OutOfMemory,
                "runtime probe read budget exhausted",
            )
        })?;
        Ok(())
    }
}
