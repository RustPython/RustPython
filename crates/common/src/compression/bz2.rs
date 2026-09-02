// spell-checker:ignore chunker libbz
//! VM-independent bzip2 stream engine.
//!
//! The engine owns its native stream state and reports plain Rust errors so
//! interpreter and embedding layers can provide their own object and exception
//! adapters.

use bzip2::{Action, Compress, Compression, Decompress, Error, Status};

use super::Chunker;

const INITIAL_BUFFER_SIZE: usize = 8192;
const BIGCHUNK: usize = 512 * 1024;

/// Double the output block until `BIGCHUNK`, then keep the size fixed.
const fn new_buffer_size(current_size: usize) -> usize {
    if current_size < BIGCHUNK {
        current_size + current_size
    } else {
        current_size
    }
}

/// Stream errors reported by the bzip2 engine. Exception mapping belongs to
/// the interpreter adapter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bz2Error {
    /// `BZ_PARAM_ERROR`
    Param,
    /// `BZ_DATA_ERROR` / `BZ_DATA_ERROR_MAGIC`
    Data,
    /// `BZ_SEQUENCE_ERROR`
    Sequence,
    /// `BZ_MEM_ERROR`
    Mem,
}

impl From<Error> for Bz2Error {
    fn from(error: Error) -> Self {
        match error {
            Error::Param => Self::Param,
            Error::Data | Error::DataMagic => Self::Data,
            Error::Sequence => Self::Sequence,
        }
    }
}

/// Incremental compressor. After `flush`, the stream is finished and must
/// not be used again.
pub struct Compressor {
    compress: Compress,
    flushed: bool,
}

impl Compressor {
    /// Create a compressor. Returns `None` when `compresslevel` is outside
    /// `1..=9`. A zero work factor selects libbz2's default of 30.
    pub fn new(compresslevel: i64) -> Option<Self> {
        let level = u32::try_from(compresslevel)
            .ok()
            .and_then(Compression::try_new)?;
        Some(Self {
            compress: Compress::new(level, 0),
            flushed: false,
        })
    }

    #[must_use]
    pub fn is_flushed(&self) -> bool {
        self.flushed
    }

    /// Compress `data` without finishing the stream.
    pub fn compress(&mut self, data: &[u8]) -> Result<Vec<u8>, Bz2Error> {
        self.run(data, Action::Run)
    }

    /// Finish the stream. The compressor must not be used again.
    pub fn flush(&mut self) -> Result<Vec<u8>, Bz2Error> {
        self.flushed = true;
        self.run(&[], Action::Finish)
    }

    /// One pass over the input with the requested action, growing the output
    /// block whenever libbz2 fills it.
    fn run(&mut self, mut input: &[u8], action: Action) -> Result<Vec<u8>, Bz2Error> {
        let mut out = Vec::new();
        let mut block = vec![0u8; INITIAL_BUFFER_SIZE];
        loop {
            // In regular compression mode, stop when input data is exhausted.
            if action == Action::Run && input.is_empty() {
                break;
            }
            let previous_in = self.compress.total_in();
            let previous_out = self.compress.total_out();
            let status = self.compress.compress(input, &mut block, action)?;
            let consumed = (self.compress.total_in() - previous_in) as usize;
            let produced = (self.compress.total_out() - previous_out) as usize;
            out.extend_from_slice(&block[..produced]);
            input = &input[consumed..];
            // In flushing mode, stop when all buffered data has been flushed.
            if action == Action::Finish && status == Status::StreamEnd {
                break;
            }
            if produced == block.len() {
                block = vec![0u8; new_buffer_size(block.len())];
            }
        }
        out.shrink_to_fit();
        Ok(out)
    }
}

/// Incremental decompressor. The first failure is latched so later calls
/// can be refused by the caller; re-entering the native stream after a
/// failure can write out of bounds.
pub struct Decompressor {
    decompress: Decompress,
    eof: bool,
    failed: bool,
    needs_input: bool,
    unused_data: Vec<u8>,
    /// Input handed to `decompress` that libbz2 has not consumed yet.
    input_buffer: Vec<u8>,
}

impl Decompressor {
    /// Create a decompressor using the fast (non-`small`) algorithm.
    #[must_use]
    pub fn new() -> Self {
        Self {
            decompress: Decompress::new(false),
            eof: false,
            failed: false,
            needs_input: true,
            unused_data: Vec::new(),
            input_buffer: Vec::new(),
        }
    }

    #[must_use]
    pub fn eof(&self) -> bool {
        self.eof
    }

    #[must_use]
    pub fn failed(&self) -> bool {
        self.failed
    }

    #[must_use]
    pub fn needs_input(&self) -> bool {
        self.needs_input
    }

    #[must_use]
    pub fn unused_data(&self) -> &[u8] {
        &self.unused_data
    }

    /// Decompress from new input and any bytes buffered by an earlier call.
    /// `max_length` of `None` means unlimited output.
    pub fn decompress(
        &mut self,
        data: &[u8],
        max_length: Option<usize>,
    ) -> Result<Vec<u8>, Bz2Error> {
        let max_length = max_length.unwrap_or(usize::MAX);
        let mut out = Vec::new();
        let mut block = vec![0u8; INITIAL_BUFFER_SIZE.min(max_length)];

        let mut failed = None;
        let mut stream_end = false;
        let leftover = {
            let mut chunks = Chunker::chain(&self.input_buffer, data);
            loop {
                let chunk = chunks.chunk();
                let previous_in = self.decompress.total_in();
                let previous_out = self.decompress.total_out();
                let status = self.decompress.decompress(chunk, &mut block);
                let consumed = (self.decompress.total_in() - previous_in) as usize;
                let produced = (self.decompress.total_out() - previous_out) as usize;
                chunks.advance(consumed);
                out.extend_from_slice(&block[..produced]);
                match status {
                    Err(error) => {
                        failed = Some(error.into());
                        break;
                    }
                    Ok(Status::MemNeeded) => {
                        failed = Some(Bz2Error::Mem);
                        break;
                    }
                    Ok(Status::StreamEnd) => {
                        stream_end = true;
                        break;
                    }
                    Ok(_) => {}
                }
                if chunks.is_empty() {
                    break;
                }
                if produced == block.len() {
                    // The output block is full: grow it unless `max_length`
                    // has already been reached.
                    if out.len() == max_length {
                        break;
                    }
                    block = vec![0u8; new_buffer_size(block.len()).min(max_length - out.len())];
                }
            }
            if chunks.is_empty() {
                None
            } else {
                Some(chunks.to_vec())
            }
        };

        if let Some(error) = failed {
            return Err(self.fail(error));
        }

        if stream_end {
            self.eof = true;
            self.needs_input = false;
            self.input_buffer.clear();
            if let Some(unused) = leftover {
                self.unused_data = unused;
            }
        } else if let Some(remaining) = leftover {
            self.needs_input = false;
            self.input_buffer = remaining;
        } else {
            self.needs_input = true;
            self.input_buffer.clear();
        }
        out.shrink_to_fit();
        Ok(out)
    }

    /// Latch the first failure and drop the pending input with it.
    fn fail(&mut self, error: Bz2Error) -> Bz2Error {
        self.failed = true;
        self.needs_input = false;
        self.input_buffer = Vec::new();
        error
    }
}

impl Default for Decompressor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8], level: i64) -> Vec<u8> {
        let mut compressor = Compressor::new(level).unwrap();
        let mut encoded = compressor.compress(data).unwrap();
        encoded.extend(compressor.flush().unwrap());
        let mut decompressor = Decompressor::new();
        let out = decompressor.decompress(&encoded, None).unwrap();
        assert!(decompressor.eof());
        out
    }

    #[test]
    fn invalid_level_is_rejected() {
        assert!(Compressor::new(0).is_none());
        assert!(Compressor::new(10).is_none());
        assert!(Compressor::new(-1).is_none());
    }

    #[test]
    fn streaming_roundtrip() {
        let data = b"the quick brown fox jumps over the lazy dog".repeat(50);
        assert_eq!(roundtrip(&data, 9), data);
    }

    #[test]
    fn unused_data_after_stream_end() {
        let mut compressor = Compressor::new(9).unwrap();
        let mut encoded = compressor.compress(b"hello").unwrap();
        encoded.extend(compressor.flush().unwrap());
        encoded.extend_from_slice(b"trailing");

        let mut decompressor = Decompressor::new();
        let out = decompressor.decompress(&encoded, None).unwrap();
        assert_eq!(out, b"hello");
        assert!(decompressor.eof());
        assert!(!decompressor.needs_input());
        assert_eq!(decompressor.unused_data(), b"trailing");
    }

    #[test]
    fn max_length_leaves_unconsumed_input() {
        let data = b"abcdefghij".repeat(20);
        let mut compressor = Compressor::new(9).unwrap();
        let mut encoded = compressor.compress(&data).unwrap();
        encoded.extend(compressor.flush().unwrap());

        let mut decompressor = Decompressor::new();
        let first = decompressor.decompress(&encoded, Some(5)).unwrap();
        assert_eq!(first.len(), 5);
        assert!(!decompressor.eof());
        assert!(!decompressor.needs_input());
        let rest = decompressor.decompress(&[], None).unwrap();
        assert_eq!([first, rest].concat(), data);
        assert!(decompressor.eof());
    }

    #[test]
    fn bad_data_latches_failure() {
        let mut decompressor = Decompressor::new();
        let err = decompressor.decompress(b"not a bz2 stream", None);
        assert_eq!(err, Err(Bz2Error::Data));
        assert!(decompressor.failed());
        assert!(!decompressor.needs_input());
        assert!(!decompressor.eof());
    }
}
