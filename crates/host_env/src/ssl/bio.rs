//! In-memory encrypted transport used by `SSLObject`.
//!
//! The unread suffix is kept as `(Vec, start)` so a read does not shift the
//! whole buffer. Compaction happens only after a substantial prefix has been
//! consumed.

/// Write rejected because [`MemoryBio::write_eof`] has already been called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBioError;

impl core::fmt::Display for MemoryBioError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("cannot write() after write_eof()")
    }
}

impl core::error::Error for MemoryBioError {}

/// In-memory BIO used by `MemoryBIO` and `SSLObject`.
#[derive(Debug, Default)]
pub struct MemoryBio {
    buffer: Vec<u8>,
    start: usize,
    eof_written: bool,
}

impl MemoryBio {
    /// Create an empty BIO.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of unread bytes.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.buffer.len() - self.start
    }

    /// True once [`write_eof`] has been called and every unread byte has been
    /// consumed.
    ///
    /// [`write_eof`]: MemoryBio::write_eof
    #[must_use]
    pub fn eof(&self) -> bool {
        self.eof_written && self.pending() == 0
    }

    /// True after [`write_eof`], even if unread bytes remain.
    ///
    /// [`write_eof`]: MemoryBio::write_eof
    #[must_use]
    pub fn eof_written(&self) -> bool {
        self.eof_written
    }

    /// Take up to `size` unread bytes.
    pub fn read(&mut self, size: usize) -> Vec<u8> {
        let count = size.min(self.pending());
        let end = self.start + count;
        let out = self.buffer[self.start..end].to_vec();
        self.start = end;
        self.compact();
        out
    }

    /// Append `data`. Fails after [`write_eof`].
    ///
    /// [`write_eof`]: MemoryBio::write_eof
    pub fn write(&mut self, data: &[u8]) -> Result<usize, MemoryBioError> {
        if self.eof_written {
            return Err(MemoryBioError);
        }
        self.compact();
        self.buffer.extend_from_slice(data);
        Ok(data.len())
    }

    /// Mark that no further writes will arrive.
    pub fn write_eof(&mut self) {
        self.eof_written = true;
    }

    fn compact(&mut self) {
        if self.start == self.buffer.len() {
            self.buffer.clear();
            self.start = 0;
        } else if self.start >= 4096 && self.start * 2 >= self.buffer.len() {
            self.buffer.copy_within(self.start.., 0);
            self.buffer.truncate(self.buffer.len() - self.start);
            self.start = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_and_eof() {
        let mut bio = MemoryBio::new();
        assert_eq!(bio.write(b"abcd").unwrap(), 4);
        assert_eq!(bio.read(2), b"ab");
        assert_eq!(bio.pending(), 2);
        bio.write_eof();
        assert!(!bio.eof());
        assert_eq!(bio.read(8), b"cd");
        assert!(bio.eof());
        assert_eq!(bio.write(b"x"), Err(MemoryBioError));
    }
}
