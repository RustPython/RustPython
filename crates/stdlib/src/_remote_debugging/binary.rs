//! Offline sampling files, compatible with CPython's version-one TACH format.
//!
//! Only the sample stream is compressible. This implementation writes raw
//! samples and explicitly rejects zstd instead of misidentifying raw data.

use alloc::sync::Arc;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

pub(crate) const FORMAT_VERSION: u32 = 1;
const MAGIC: u32 = 0x5441_4348;
const HEADER_SIZE: usize = 64;
const FOOTER_SIZE: usize = 32;
const STATS_SIZE: usize = 56;
const STACK_LIMIT: usize = 256;
const WRITE_BUFFER_SIZE: usize = 512 * 1024;
const REPEAT_BUFFER_SIZE: usize = 16 * 1024;
const FULL: u8 = 1;
const SUFFIX: u8 = 2;
const POP_PUSH: u8 = 3;
const REPEAT: u8 = 0;

#[derive(Debug)]
pub(crate) enum Error {
    Io(io::Error),
    Value(String),
    Overflow(&'static str),
    Memory,
    Runtime(&'static str),
}

pub(crate) type Result<T> = core::result::Result<T, Error>;

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Value(message.into())
}

fn reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<()> {
    let capacity = values
        .len()
        .checked_add(additional)
        .ok_or(Error::Overflow("Array capacity overflow"))?;
    let allocation = capacity
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(Error::Overflow("Array allocation size overflow"))?;
    if allocation > isize::MAX as usize {
        return Err(Error::Overflow("Array allocation size overflow"));
    }
    values.try_reserve(additional).map_err(|_| Error::Memory)
}

fn copy_string(value: &str) -> Result<String> {
    let mut result = String::new();
    result.try_reserve(value.len()).map_err(|_| Error::Memory)?;
    result.push_str(value);
    Ok(result)
}

fn push_varint(bytes: &mut Vec<u8>, mut value: u64) -> Result<()> {
    reserve(bytes, 10)?;
    while value >= 0x80 {
        bytes.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
    Ok(())
}

fn push_signed(bytes: &mut Vec<u8>, value: i32) -> Result<()> {
    push_varint(
        bytes,
        (((value as u32) << 1) ^ ((value >> 31) as u32)).into(),
    )
}

fn add(counter: &mut u64, value: u64) -> Result<()> {
    *counter = counter
        .checked_add(value)
        .ok_or(Error::Overflow("Sample statistics overflow"))?;
    Ok(())
}

#[derive(Debug)]
pub(crate) struct Frame {
    pub(crate) filename: String,
    pub(crate) funcname: String,
    pub(crate) location: [i32; 4],
    pub(crate) opcode: u8,
}

#[derive(Debug)]
pub(crate) struct Sample {
    pub(crate) interpreter_id: u32,
    pub(crate) thread_id: u64,
    pub(crate) status: u8,
    pub(crate) frames: Vec<Frame>,
    pub(crate) timestamp_us: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProfileStats {
    pub(crate) duration_sec: f64,
    pub(crate) sample_rate: f64,
    pub(crate) error_rate: Option<f64>,
    pub(crate) missed_samples: Option<f64>,
}

impl ProfileStats {
    fn validate(&self) -> Result<()> {
        if !self.duration_sec.is_finite() || self.duration_sec < 0.0 {
            return Err(invalid("duration must be a finite non-negative value"));
        }
        if !self.sample_rate.is_finite() || self.sample_rate < 0.0 {
            return Err(invalid("sample rate must be a finite non-negative value"));
        }
        if self
            .error_rate
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err(invalid("error rate must be a finite non-negative value"));
        }
        if self.missed_samples.is_some_and(|value| !value.is_finite()) {
            return Err(invalid("missed samples must be a finite value"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WriterStats {
    pub(crate) repeat_records: u64,
    pub(crate) repeat_samples: u64,
    pub(crate) full_records: u64,
    pub(crate) suffix_records: u64,
    pub(crate) pop_push_records: u64,
    pub(crate) total_samples: u64,
    pub(crate) total_frames_written: u64,
    pub(crate) frames_saved: u64,
    pub(crate) bytes_written: u64,
}

impl WriterStats {
    pub(crate) fn total_records(&self) -> u64 {
        self.repeat_records + self.full_records + self.suffix_records + self.pop_push_records
    }

    pub(crate) fn frame_compression_pct(&self) -> f64 {
        let potential = self.total_frames_written as f64 + self.frames_saved as f64;
        if potential == 0.0 {
            0.0
        } else {
            self.frames_saved as f64 / potential * 100.0
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ReaderStats {
    pub(crate) repeat_records: u64,
    pub(crate) repeat_samples: u64,
    pub(crate) full_records: u64,
    pub(crate) suffix_records: u64,
    pub(crate) pop_push_records: u64,
    pub(crate) total_samples: u64,
    pub(crate) stack_reconstructions: u64,
}

impl ReaderStats {
    pub(crate) fn total_records(&self) -> u64 {
        self.repeat_records + self.full_records + self.suffix_records + self.pop_push_records
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FrameEntry {
    filename: u32,
    funcname: u32,
    location: [i32; 4],
    opcode: u8,
}

#[derive(Debug)]
struct WriterThread {
    thread_id: u64,
    interpreter_id: u32,
    timestamp: u64,
    stack: [u32; STACK_LIMIT],
    depth: usize,
    pending: Vec<u8>,
    pending_count: u32,
}

#[derive(Debug)]
pub(crate) struct Writer {
    file: Option<File>,
    buffer: Vec<u8>,
    strings: Vec<String>,
    string_indices: HashMap<String, u32>,
    frames: Vec<FrameEntry>,
    frame_indices: HashMap<FrameEntry, u32>,
    threads: Vec<WriterThread>,
    thread_indices: HashMap<(u64, u32), usize>,
    interval: u64,
    start: u64,
    compression: i32,
    mode: i32,
    features: i32,
    python_version: [u8; 3],
    profile: Option<ProfileStats>,
    stats: WriterStats,
}

impl Writer {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create(
        path: &Path,
        interval: u64,
        start: u64,
        compression: i32,
        mode: i32,
        features: i32,
        python_version: [u8; 3],
    ) -> Result<Self> {
        if !(-1..=4).contains(&mode) {
            return Err(invalid("invalid profiling mode"));
        }
        if !(-1..=31).contains(&features) {
            return Err(invalid("invalid capture features"));
        }
        if compression == 1 {
            return Err(Error::Runtime(
                "zstd compression requested but not available (HAVE_ZSTD not defined)",
            ));
        }
        let mut buffer = Vec::new();
        reserve(&mut buffer, WRITE_BUFFER_SIZE)?;
        let mut file = rustpython_host_env::fs::create(path)?;
        file.write_all(&[0; HEADER_SIZE])?;
        Ok(Self {
            file: Some(file),
            buffer,
            strings: Vec::new(),
            string_indices: HashMap::new(),
            frames: Vec::new(),
            frame_indices: HashMap::new(),
            threads: Vec::new(),
            thread_indices: HashMap::new(),
            interval,
            start,
            compression,
            mode,
            features,
            python_version,
            profile: None,
            stats: WriterStats::default(),
        })
    }

    fn file(&mut self) -> Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| invalid("Writer is closed"))
    }

    fn flush_buffer(&mut self) -> Result<()> {
        if !self.buffer.is_empty() {
            self.file
                .as_mut()
                .ok_or_else(|| invalid("Writer is closed"))?
                .write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }

    fn write_bytes(&mut self, mut bytes: &[u8]) -> Result<()> {
        let length = bytes.len() as u64;
        while !bytes.is_empty() {
            let count = bytes.len().min(WRITE_BUFFER_SIZE - self.buffer.len());
            self.buffer.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.buffer.len() == WRITE_BUFFER_SIZE {
                self.flush_buffer()?;
            }
        }
        add(&mut self.stats.bytes_written, length)
    }

    fn intern_string(&mut self, value: &str) -> Result<u32> {
        if let Some(index) = self.string_indices.get(value) {
            return Ok(*index);
        }
        if self.strings.len() >= u32::MAX as usize {
            return Err(Error::Overflow("too many strings for binary format"));
        }
        if value.len() > u32::MAX as usize {
            return Err(Error::Overflow(
                "string length exceeds binary format maximum",
            ));
        }
        reserve(&mut self.strings, 1)?;
        self.string_indices
            .try_reserve(1)
            .map_err(|_| Error::Memory)?;
        let stored = copy_string(value)?;
        let key = copy_string(value)?;
        let index = self.strings.len() as u32;
        self.strings.push(stored);
        self.string_indices.insert(key, index);
        Ok(index)
    }

    fn intern_frame(&mut self, frame: &Frame) -> Result<u32> {
        let entry = FrameEntry {
            filename: self.intern_string(&frame.filename)?,
            funcname: self.intern_string(&frame.funcname)?,
            location: frame.location,
            opcode: frame.opcode,
        };
        if let Some(index) = self.frame_indices.get(&entry) {
            return Ok(*index);
        }
        if self.frames.len() >= u32::MAX as usize {
            return Err(Error::Overflow("too many frames for binary format"));
        }
        // Reject unrepresentable deltas before adding a frame to the table.
        location_delta(entry.location[0], entry.location[1])?;
        location_delta(entry.location[2], entry.location[3])?;
        reserve(&mut self.frames, 1)?;
        self.frame_indices
            .try_reserve(1)
            .map_err(|_| Error::Memory)?;
        let index = self.frames.len() as u32;
        self.frames.push(entry);
        self.frame_indices.insert(entry, index);
        Ok(index)
    }

    fn thread(&mut self, thread_id: u64, interpreter_id: u32) -> Result<usize> {
        let key = (thread_id, interpreter_id);
        if let Some(index) = self.thread_indices.get(&key) {
            return Ok(*index);
        }
        if self.threads.len() >= u32::MAX as usize {
            return Err(Error::Overflow("too many threads for binary format"));
        }
        reserve(&mut self.threads, 1)?;
        self.thread_indices
            .try_reserve(1)
            .map_err(|_| Error::Memory)?;
        let index = self.threads.len();
        self.threads.push(WriterThread {
            thread_id,
            interpreter_id,
            timestamp: self.start,
            stack: [0; STACK_LIMIT],
            depth: 0,
            pending: Vec::new(),
            pending_count: 0,
        });
        self.thread_indices.insert(key, index);
        Ok(index)
    }

    fn flush_repeat(&mut self, index: usize) -> Result<()> {
        let thread = &self.threads[index];
        if thread.pending_count == 0 {
            return Ok(());
        }
        let count = thread.pending_count;
        let saved = u64::from(count) * thread.depth as u64;
        let mut header = record_header(thread.thread_id, thread.interpreter_id, REPEAT)?;
        push_varint(&mut header, count.into())?;
        self.write_bytes(&header)?;
        let mut pending = core::mem::take(&mut self.threads[index].pending);
        let result = self.write_bytes(&pending);
        if result.is_ok() {
            pending.clear();
            self.threads[index].pending_count = 0;
        }
        self.threads[index].pending = pending;
        result?;
        add(&mut self.stats.repeat_records, 1)?;
        add(&mut self.stats.repeat_samples, count.into())?;
        add(&mut self.stats.frames_saved, saved)
    }

    pub(crate) fn write_sample(&mut self, sample: &Sample) -> Result<()> {
        self.file()?;
        let mut stack = [0; STACK_LIMIT];
        let depth = sample.frames.len().min(STACK_LIMIT);
        for (slot, frame) in stack.iter_mut().zip(sample.frames.iter()).take(depth) {
            *slot = self.intern_frame(frame)?;
        }
        let index = self.thread(sample.thread_id, sample.interpreter_id)?;
        let thread = &self.threads[index];
        let delta = sample.timestamp_us.wrapping_sub(thread.timestamp);
        let previous = &thread.stack[..thread.depth];
        let current = &stack[..depth];
        if previous == current {
            if thread.pending.len() + 11 > REPEAT_BUFFER_SIZE {
                self.flush_repeat(index)?;
            }
            let thread = &mut self.threads[index];
            reserve(&mut thread.pending, 11)?;
            push_varint(&mut thread.pending, delta)?;
            thread.pending.push(sample.status);
            thread.pending_count += 1;
        } else {
            let shared = previous
                .iter()
                .rev()
                .zip(current.iter().rev())
                .take_while(|(left, right)| left == right)
                .count();
            let pop = previous.len() - shared;
            let push = depth - shared;
            let encoding = if shared == 0 {
                FULL
            } else if pop == 0 && push > 0 {
                SUFFIX
            } else if shared < depth / 2 {
                FULL
            } else {
                POP_PUSH
            };
            self.flush_repeat(index)?;
            let mut record = record_header(sample.thread_id, sample.interpreter_id, encoding)?;
            push_varint(&mut record, delta)?;
            reserve(&mut record, 1)?;
            record.push(sample.status);
            let written = match encoding {
                FULL => {
                    push_varint(&mut record, depth as u64)?;
                    depth
                }
                SUFFIX => {
                    push_varint(&mut record, shared as u64)?;
                    push_varint(&mut record, push as u64)?;
                    push
                }
                _ => {
                    push_varint(&mut record, pop as u64)?;
                    push_varint(&mut record, push as u64)?;
                    push
                }
            };
            for &frame in &stack[..written] {
                push_varint(&mut record, frame.into())?;
            }
            self.write_bytes(&record)?;
            match encoding {
                FULL => add(&mut self.stats.full_records, 1)?,
                SUFFIX => add(&mut self.stats.suffix_records, 1)?,
                _ => add(&mut self.stats.pop_push_records, 1)?,
            }
            add(&mut self.stats.total_frames_written, written as u64)?;
            if encoding != FULL {
                add(&mut self.stats.frames_saved, shared as u64)?;
            }
            self.threads[index].stack = stack;
            self.threads[index].depth = depth;
        }
        self.threads[index].timestamp = sample.timestamp_us;
        add(&mut self.stats.total_samples, 1)
    }

    pub(crate) fn set_stats(&mut self, stats: ProfileStats) -> Result<()> {
        self.file()?;
        stats.validate()?;
        self.profile = Some(stats);
        Ok(())
    }

    pub(crate) fn stats(&self) -> &WriterStats {
        &self.stats
    }

    pub(crate) fn total_samples(&self) -> u64 {
        self.stats.total_samples
    }

    pub(crate) fn finalize(&mut self) -> Result<()> {
        if self.file.is_none() {
            return Err(invalid("Writer is already closed"));
        }
        for index in 0..self.threads.len() {
            self.flush_repeat(index)?;
        }
        self.flush_buffer()?;
        let string_offset = self.file()?.stream_position()?;
        let mut scratch = Vec::new();
        reserve(&mut scratch, 64)?;
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| invalid("Writer is closed"))?;
        for string in &self.strings {
            scratch.clear();
            push_varint(&mut scratch, string.len() as u64)?;
            file.write_all(&scratch)?;
            file.write_all(string.as_bytes())?;
        }
        let frame_offset = file.stream_position()?;
        for frame in &self.frames {
            scratch.clear();
            push_varint(&mut scratch, frame.filename.into())?;
            push_varint(&mut scratch, frame.funcname.into())?;
            push_signed(&mut scratch, frame.location[0])?;
            push_signed(
                &mut scratch,
                location_delta(frame.location[0], frame.location[1])?,
            )?;
            push_signed(&mut scratch, frame.location[2])?;
            push_signed(
                &mut scratch,
                location_delta(frame.location[2], frame.location[3])?,
            )?;
            reserve(&mut scratch, 1)?;
            scratch.push(frame.opcode);
            file.write_all(&scratch)?;
        }
        if let Some(stats) = self.profile {
            let mut bytes = [0; STATS_SIZE];
            bytes[0..8].copy_from_slice(&stats.duration_sec.to_ne_bytes());
            bytes[8..16].copy_from_slice(&stats.sample_rate.to_ne_bytes());
            bytes[16..24].copy_from_slice(&stats.error_rate.unwrap_or(0.0).to_ne_bytes());
            bytes[24..32].copy_from_slice(&stats.missed_samples.unwrap_or(0.0).to_ne_bytes());
            let present = u32::from(stats.error_rate.is_some())
                | (u32::from(stats.missed_samples.is_some()) << 1);
            bytes[32..36].copy_from_slice(&present.to_ne_bytes());
            bytes[40..48].copy_from_slice(b"TACHSTAT");
            bytes[48..52].copy_from_slice(&1_u32.to_ne_bytes());
            bytes[52..56].copy_from_slice(&(STATS_SIZE as u32).to_ne_bytes());
            file.write_all(&bytes)?;
        }
        let file_size = file
            .stream_position()?
            .checked_add(FOOTER_SIZE as u64)
            .ok_or(Error::Overflow("File size exceeds binary format maximum"))?;
        let mut footer = [0; FOOTER_SIZE];
        footer[0..4].copy_from_slice(&(self.strings.len() as u32).to_ne_bytes());
        footer[4..8].copy_from_slice(&(self.frames.len() as u32).to_ne_bytes());
        footer[8..16].copy_from_slice(&file_size.to_ne_bytes());
        file.write_all(&footer)?;
        let mut header = [0; HEADER_SIZE];
        header[0..4].copy_from_slice(&MAGIC.to_ne_bytes());
        header[4..8].copy_from_slice(&FORMAT_VERSION.to_ne_bytes());
        header[8..11].copy_from_slice(&self.python_version);
        header[12..20].copy_from_slice(&self.start.to_ne_bytes());
        header[20..28].copy_from_slice(&self.interval.to_ne_bytes());
        header[28..36].copy_from_slice(&self.stats.total_samples.to_ne_bytes());
        header[36..40].copy_from_slice(&(self.threads.len() as u32).to_ne_bytes());
        header[40..48].copy_from_slice(&string_offset.to_ne_bytes());
        header[48..56].copy_from_slice(&frame_offset.to_ne_bytes());
        header[56..60].copy_from_slice(&(self.compression as u32).to_ne_bytes());
        let mut config = (self.mode + 1) as u32;
        if self.features >= 0 {
            config |= 8 | ((self.features as u32) << 4);
        }
        header[60..64].copy_from_slice(&config.to_ne_bytes());
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header)?;
        file.flush()?;
        self.file.take();
        Ok(())
    }
}

fn location_delta(start: i32, end: i32) -> Result<i32> {
    if start == -1 || end == -1 {
        Ok(0)
    } else {
        end.checked_sub(start)
            .ok_or(Error::Overflow("Source location delta exceeds i32 range"))
    }
}

fn record_header(thread: u64, interpreter: u32, encoding: u8) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reserve(&mut bytes, 32)?;
    bytes.extend_from_slice(&thread.to_ne_bytes());
    bytes.extend_from_slice(&interpreter.to_ne_bytes());
    bytes.push(encoding);
    Ok(bytes)
}

#[derive(Clone, Copy, Debug)]
enum Endian {
    Little,
    Big,
}

impl Endian {
    fn u32(self, bytes: [u8; 4]) -> u32 {
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }

    fn u64(self, bytes: [u8; 8]) -> u64 {
        match self {
            Self::Little => u64::from_le_bytes(bytes),
            Self::Big => u64::from_be_bytes(bytes),
        }
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.offset
    }

    fn bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| invalid("Unexpected end of binary data"))?;
        let result = self
            .data
            .get(self.offset..end)
            .ok_or_else(|| invalid("Unexpected end of binary data"))?;
        self.offset = end;
        Ok(result)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.bytes(N)?
            .try_into()
            .map_err(|_| invalid("Unexpected end of binary data"))
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.fixed::<1>()?[0])
    }

    fn varint(&mut self) -> Result<u64> {
        let mut pos = self.offset;
        let mut value = 0_u64;
        for shift in (0..=63).step_by(7) {
            let byte = *self
                .data
                .get(pos)
                .ok_or_else(|| invalid("Invalid or incomplete varint in binary data"))?;
            pos += 1;
            if shift == 63 && byte & 0xfe != 0 {
                return Err(invalid("Invalid or incomplete varint in binary data"));
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                self.offset = pos;
                return Ok(value);
            }
        }
        Err(invalid("Invalid or incomplete varint in binary data"))
    }

    fn var_u32(&mut self) -> Result<u32> {
        let previous = self.offset;
        let value = self.varint()?;
        match u32::try_from(value) {
            Ok(value) => Ok(value),
            Err(_) => {
                self.offset = previous;
                Err(invalid("Invalid or incomplete varint in binary data"))
            }
        }
    }

    fn signed(&mut self) -> Result<i32> {
        let value = self.var_u32()?;
        Ok(((value >> 1) as i32) ^ -((value & 1) as i32))
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Metadata {
    pub(crate) version: u32,
    pub(crate) python_version: [u8; 3],
    pub(crate) start_time_us: u64,
    pub(crate) sample_interval_us: u64,
    pub(crate) sample_count: u64,
    pub(crate) thread_count: u32,
    pub(crate) string_count: u32,
    pub(crate) frame_count: u32,
    pub(crate) compression_type: u32,
    pub(crate) mode: Option<i32>,
    pub(crate) capture_features: Option<i32>,
    pub(crate) profile_stats: Option<ProfileStats>,
}

/// The snapshot and parsed tables use memory proportional to actual file bytes,
/// not header sample counts. Repeat timestamps are expanded only one batch at a
/// time, and every batch is bounded by encoded bytes remaining in the snapshot.
#[derive(Debug)]
pub(crate) struct Reader {
    data: Vec<u8>,
    sample_end: usize,
    endian: Endian,
    strings: Vec<String>,
    frames: Vec<FrameEntry>,
    metadata: Metadata,
}

impl Reader {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let mut file = rustpython_host_env::fs::open(path)?;
        let mut data = Vec::new();
        let mut chunk = [0; 64 * 1024];
        loop {
            let count = match file.read(&mut chunk) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                break;
            }
            reserve(&mut data, count)?;
            data.extend_from_slice(&chunk[..count]);
        }
        Self::parse(data)
    }

    fn parse(data: Vec<u8>) -> Result<Self> {
        if data.len() < HEADER_SIZE {
            return Err(invalid("File too small for header"));
        }
        if data.len() < HEADER_SIZE + FOOTER_SIZE {
            return Err(invalid("File too small for footer"));
        }
        let mut header = Cursor::new(&data[..HEADER_SIZE]);
        let magic = header.fixed::<4>()?;
        let endian = if u32::from_le_bytes(magic) == MAGIC {
            Endian::Little
        } else if u32::from_be_bytes(magic) == MAGIC {
            Endian::Big
        } else {
            return Err(invalid(format!(
                "Invalid magic number: 0x{:08x}",
                u32::from_ne_bytes(magic)
            )));
        };
        let version = endian.u32(header.fixed()?);
        let python_version = header.fixed::<3>()?;
        header.byte()?;
        if version != FORMAT_VERSION {
            return Err(invalid(if version > FORMAT_VERSION {
                format!(
                    "Binary file was created with Python {}.{}.{} (format version {}), \
                     but this reader supports format version {}",
                    python_version[0],
                    python_version[1],
                    python_version[2],
                    version,
                    FORMAT_VERSION
                )
            } else {
                format!(
                    "Unsupported format version {version} (this reader supports version {FORMAT_VERSION})"
                )
            }));
        }
        let start_time_us = endian.u64(header.fixed()?);
        let sample_interval_us = endian.u64(header.fixed()?);
        let sample_count = endian.u64(header.fixed()?);
        let thread_count = endian.u32(header.fixed()?);
        let string_offset = endian.u64(header.fixed()?);
        let frame_offset = endian.u64(header.fixed()?);
        let compression_type = endian.u32(header.fixed()?);
        let config = endian.u32(header.fixed()?);
        let mode = config & 7;
        if mode > 5 {
            return Err(invalid(format!("Invalid profiling mode in header: {mode}")));
        }
        let footer_start = data.len() - FOOTER_SIZE;
        let mut footer = Cursor::new(&data[footer_start..]);
        let string_count = endian.u32(footer.fixed()?);
        let frame_count = endian.u32(footer.fixed()?);
        let (profile_stats, _) = parse_profile_stats(&data, endian, footer_start)?;
        for (name, offset) in [("string", string_offset), ("frame", frame_offset)] {
            if offset > data.len() as u64 {
                return Err(invalid(format!(
                    "Invalid {name} table offset: {offset} exceeds file size {}",
                    data.len()
                )));
            }
            if offset < HEADER_SIZE as u64 {
                return Err(invalid(format!(
                    "Invalid {name} table offset: {offset} is before data section"
                )));
            }
        }
        if string_offset > frame_offset {
            return Err(invalid(format!(
                "Invalid table offsets: string table ({string_offset}) is after frame table ({frame_offset})"
            )));
        }
        let string_offset = usize::try_from(string_offset)
            .map_err(|_| Error::Overflow("File is too large to read"))?;
        let frame_offset = usize::try_from(frame_offset)
            .map_err(|_| Error::Overflow("File is too large to read"))?;
        if compression_type == 1 {
            return Err(Error::Runtime(
                "File uses zstd compression but zstd support not compiled in",
            ));
        }
        // CPython bounds table entries by the remaining file bytes, including
        // subsequent sections. Keep the count and cursor checks anchored to EOF.
        let mut string_data = Cursor::new(&data[string_offset..]);
        validate_count("string", string_count, string_data.remaining(), 1)?;
        let mut strings = Vec::new();
        reserve(&mut strings, string_count as usize)?;
        for _ in 0..string_count {
            let length = string_data
                .var_u32()
                .map_err(|_| invalid("Malformed varint in string table"))?;
            let bytes = string_data
                .bytes(length as usize)
                .map_err(|_| invalid("String table overflow"))?;
            strings.push(decode_utf8_replacing(bytes)?);
        }
        let mut frame_data = Cursor::new(&data[frame_offset..]);
        validate_count("frame", frame_count, frame_data.remaining(), 7)?;
        let mut frames = Vec::new();
        reserve(&mut frames, frame_count as usize)?;
        for _ in 0..frame_count {
            let filename = frame_data
                .var_u32()
                .map_err(|_| invalid("Malformed varint in frame table (filename)"))?;
            let funcname = frame_data
                .var_u32()
                .map_err(|_| invalid("Malformed varint in frame table (funcname)"))?;
            let line = frame_data
                .signed()
                .map_err(|_| invalid("Malformed varint in frame table (lineno)"))?;
            let line_delta = frame_data
                .signed()
                .map_err(|_| invalid("Malformed varint in frame table (end_lineno_delta)"))?;
            let column = frame_data
                .signed()
                .map_err(|_| invalid("Malformed varint in frame table (column)"))?;
            let column_delta = frame_data
                .signed()
                .map_err(|_| invalid("Malformed varint in frame table (end_column_delta)"))?;
            let opcode = frame_data
                .byte()
                .map_err(|_| invalid("Unexpected end of frame table (opcode)"))?;
            frames.push(FrameEntry {
                filename,
                funcname,
                location: [
                    line,
                    reconstruct_location(line, line_delta)?,
                    column,
                    reconstruct_location(column, column_delta)?,
                ],
                opcode,
            });
        }
        Ok(Self {
            data,
            sample_end: string_offset,
            endian,
            strings,
            frames,
            metadata: Metadata {
                version,
                python_version,
                start_time_us,
                sample_interval_us,
                sample_count,
                thread_count,
                string_count,
                frame_count,
                compression_type,
                mode: (mode != 0).then_some(mode as i32 - 1),
                capture_features: (config & 8 != 0).then_some(((config >> 4) & 31) as i32),
                profile_stats,
            },
        })
    }

    pub(crate) fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub(crate) fn replay(self: &Arc<Self>) -> Replay {
        Replay {
            reader: Arc::clone(self),
            offset: HEADER_SIZE,
            threads: Vec::new(),
            thread_indices: HashMap::new(),
            repeat: None,
            stats: ReaderStats::default(),
            finished: false,
        }
    }

    fn build_sample(&self, thread: &ReaderThread, status: u8, timestamp_us: u64) -> Result<Sample> {
        let mut frames = Vec::new();
        reserve(&mut frames, thread.depth)?;
        for &index in &thread.stack[..thread.depth] {
            let frame = self
                .frames
                .get(index as usize)
                .ok_or_else(|| invalid(format!("Invalid frame index: {index}")))?;
            let filename = self
                .strings
                .get(frame.filename as usize)
                .ok_or_else(|| invalid("Invalid string index in frame"))?;
            let funcname = self
                .strings
                .get(frame.funcname as usize)
                .ok_or_else(|| invalid("Invalid string index in frame"))?;
            frames.push(Frame {
                filename: copy_string(filename)?,
                funcname: copy_string(funcname)?,
                location: frame.location,
                opcode: frame.opcode,
            });
        }
        Ok(Sample {
            interpreter_id: thread.interpreter_id,
            thread_id: thread.thread_id,
            status,
            frames,
            timestamp_us,
        })
    }
}

fn reconstruct_location(start: i32, delta: i32) -> Result<i32> {
    if start == -1 {
        Ok(-1)
    } else {
        start
            .checked_add(delta)
            .ok_or_else(|| invalid("Source location exceeds i32 range"))
    }
}

fn validate_count(name: &str, count: u32, available: usize, minimum: usize) -> Result<()> {
    let maximum = available / minimum;
    if count as u64 > maximum as u64 {
        return Err(invalid(format!(
            "Invalid {name} count {count} exceeds maximum possible {maximum}"
        )));
    }
    Ok(())
}

fn decode_utf8_replacing(mut bytes: &[u8]) -> Result<String> {
    let mut result = String::new();
    while !bytes.is_empty() {
        match core::str::from_utf8(bytes) {
            Ok(valid) => {
                result.try_reserve(valid.len()).map_err(|_| Error::Memory)?;
                result.push_str(valid);
                break;
            }
            Err(error) => {
                let count = error.valid_up_to();
                let valid = core::str::from_utf8(&bytes[..count])
                    .map_err(|_| invalid("Invalid UTF-8 string boundary"))?;
                let additional = count.checked_add(3).ok_or(Error::Memory)?;
                result.try_reserve(additional).map_err(|_| Error::Memory)?;
                result.push_str(valid);
                result.push('\u{fffd}');
                let skip = error.error_len().unwrap_or(bytes.len() - count);
                bytes = &bytes[count + skip..];
            }
        }
    }
    Ok(result)
}

fn parse_profile_stats(
    data: &[u8],
    endian: Endian,
    footer_start: usize,
) -> Result<(Option<ProfileStats>, usize)> {
    if footer_start < 32 || &data[footer_start - 16..footer_start - 8] != b"TACHSTAT" {
        return Ok((None, footer_start));
    }
    let mut tail = Cursor::new(&data[footer_start - 8..footer_start]);
    let version = endian.u32(tail.fixed()?);
    let size = endian.u32(tail.fixed()?) as usize;
    if size < 32 || size > footer_start {
        return Err(invalid("Invalid profiling statistics size"));
    }
    let start = footer_start - size;
    if start < HEADER_SIZE {
        return Err(invalid("Invalid profiling statistics size"));
    }
    if version != 1 {
        return Ok((None, start));
    }
    let mut cursor = Cursor::new(&data[start..footer_start]);
    let duration_sec = f64::from_bits(endian.u64(cursor.fixed()?));
    let sample_rate = f64::from_bits(endian.u64(cursor.fixed()?));
    let (error_rate, missed_samples) = if size >= STATS_SIZE {
        let error_rate = f64::from_bits(endian.u64(cursor.fixed()?));
        let missed_samples = f64::from_bits(endian.u64(cursor.fixed()?));
        let present = endian.u32(cursor.fixed()?);
        (
            (present & 1 != 0).then_some(error_rate),
            (present & 2 != 0).then_some(missed_samples),
        )
    } else {
        (None, None)
    };
    let stats = ProfileStats {
        duration_sec,
        sample_rate,
        error_rate,
        missed_samples,
    };
    stats
        .validate()
        .map_err(|_| invalid("Invalid profiling statistics values"))?;
    Ok((Some(stats), start))
}

#[derive(Debug)]
struct ReaderThread {
    thread_id: u64,
    interpreter_id: u32,
    timestamp: u64,
    stack: [u32; STACK_LIMIT],
    depth: usize,
}

#[derive(Clone, Copy, Debug)]
struct RepeatState {
    thread: usize,
    remaining: u32,
    total: u32,
}

#[derive(Debug)]
pub(crate) struct Batch {
    pub(crate) sample: Sample,
    pub(crate) timestamps: Vec<u64>,
    pub(crate) is_repeat: bool,
    pub(crate) record_complete: bool,
    pub(crate) record_samples: u32,
}

/// Independent replay state keeps close and Python callbacks from invalidating
/// any borrowed file data. No sample-count-sized allocation is performed.
#[derive(Debug)]
pub(crate) struct Replay {
    reader: Arc<Reader>,
    offset: usize,
    threads: Vec<ReaderThread>,
    thread_indices: HashMap<(u64, u32), usize>,
    repeat: Option<RepeatState>,
    stats: ReaderStats,
    finished: bool,
}

impl Replay {
    pub(crate) fn stats(&self) -> &ReaderStats {
        &self.stats
    }

    pub(crate) fn replayed(&self) -> u64 {
        self.stats.total_samples
    }

    fn thread(&mut self, thread_id: u64, interpreter_id: u32) -> Result<usize> {
        let key = (thread_id, interpreter_id);
        if let Some(index) = self.thread_indices.get(&key) {
            return Ok(*index);
        }
        if self.threads.len() as u64 >= u64::from(self.reader.metadata.thread_count) {
            return Err(invalid(format!(
                "Invalid thread count: sample data contains more unique threads than declared in header \
                 (declared {}, found at least {})",
                self.reader.metadata.thread_count,
                self.threads.len() + 1
            )));
        }
        reserve(&mut self.threads, 1)?;
        self.thread_indices
            .try_reserve(1)
            .map_err(|_| Error::Memory)?;
        let index = self.threads.len();
        self.threads.push(ReaderThread {
            thread_id,
            interpreter_id,
            timestamp: self.reader.metadata.start_time_us,
            stack: [0; STACK_LIMIT],
            depth: 0,
        });
        self.thread_indices.insert(key, index);
        Ok(index)
    }

    pub(crate) fn next_batch(&mut self) -> Result<Option<Batch>> {
        if self.finished {
            return Ok(None);
        }
        let result = self.next_batch_inner();
        if result.is_err() {
            self.finished = true;
        }
        result
    }

    fn next_batch_inner(&mut self) -> Result<Option<Batch>> {
        loop {
            if let Some(repeat) = self.repeat {
                return self.repeat_batch(repeat).map(Some);
            }
            if self.offset == self.reader.sample_end {
                if self.stats.total_samples != self.reader.metadata.sample_count {
                    return Err(invalid(format!(
                        "Sample count mismatch: header declares {} samples but replay decoded {}",
                        self.reader.metadata.sample_count, self.stats.total_samples
                    )));
                }
                self.finished = true;
                return Ok(None);
            }
            let remaining = self.reader.sample_end - self.offset;
            if remaining < 13 {
                return Err(invalid(format!(
                    "Truncated sample data: {remaining} trailing bytes"
                )));
            }
            let mut cursor = Cursor::new(&self.reader.data[self.offset..self.reader.sample_end]);
            let thread_id = self.reader.endian.u64(cursor.fixed()?);
            let interpreter_id = self.reader.endian.u32(cursor.fixed()?);
            let encoding = cursor.byte()?;
            self.offset += cursor.offset;
            let index = self.thread(thread_id, interpreter_id)?;
            let mut cursor = Cursor::new(&self.reader.data[self.offset..self.reader.sample_end]);
            if encoding == REPEAT {
                let count = cursor
                    .var_u32()
                    .map_err(|_| invalid("Malformed varint for RLE count"))?;
                validate_count("RLE", count, cursor.remaining(), 2)?;
                validate_sample_count(self.stats.total_samples, u64::from(count))?;
                self.offset += cursor.offset;
                add(&mut self.stats.repeat_records, 1)?;
                add(&mut self.stats.repeat_samples, count.into())?;
                if count != 0 {
                    self.repeat = Some(RepeatState {
                        thread: index,
                        remaining: count,
                        total: count,
                    });
                }
                continue;
            }
            if ![FULL, SUFFIX, POP_PUSH].contains(&encoding) {
                return Err(invalid(format!("Unknown stack encoding: {encoding}")));
            }
            let delta = cursor
                .varint()
                .map_err(|_| invalid("Malformed varint in sample data"))?;
            let status = cursor
                .byte()
                .map_err(|_| invalid("Unexpected end of sample data"))?;
            let thread = &mut self.threads[index];
            decode_stack(thread, encoding, &mut cursor)?;
            thread.timestamp = thread.timestamp.wrapping_add(delta);
            self.offset += cursor.offset;
            match encoding {
                FULL => add(&mut self.stats.full_records, 1)?,
                SUFFIX => add(&mut self.stats.suffix_records, 1)?,
                _ => add(&mut self.stats.pop_push_records, 1)?,
            }
            add(&mut self.stats.stack_reconstructions, 1)?;
            validate_sample_count(self.stats.total_samples, 1)?;
            let sample = self.reader.build_sample(thread, status, thread.timestamp)?;
            let mut timestamps = Vec::new();
            reserve(&mut timestamps, 1)?;
            timestamps.push(thread.timestamp);
            add(&mut self.stats.total_samples, 1)?;
            return Ok(Some(Batch {
                sample,
                timestamps,
                is_repeat: false,
                record_complete: true,
                record_samples: 1,
            }));
        }
    }

    fn repeat_batch(&mut self, mut repeat: RepeatState) -> Result<Batch> {
        let mut cursor = Cursor::new(&self.reader.data[self.offset..self.reader.sample_end]);
        let thread = &mut self.threads[repeat.thread];
        let mut timestamps = Vec::new();
        let mut batch_status = None;
        while repeat.remaining > 0 {
            let previous = cursor.offset;
            let delta = cursor
                .varint()
                .map_err(|_| invalid("Malformed varint in RLE sample data"))?;
            let status = cursor
                .byte()
                .map_err(|_| invalid("Unexpected end of sample data in RLE"))?;
            if batch_status.is_some_and(|previous| previous != status) {
                cursor.offset = previous;
                break;
            }
            reserve(&mut timestamps, 1)?;
            batch_status = Some(status);
            thread.timestamp = thread.timestamp.wrapping_add(delta);
            timestamps.push(thread.timestamp);
            repeat.remaining -= 1;
        }
        let status = batch_status.ok_or_else(|| invalid("Empty RLE batch"))?;
        let timestamp = *timestamps
            .first()
            .ok_or_else(|| invalid("Empty RLE batch"))?;
        let sample = self.reader.build_sample(thread, status, timestamp)?;
        self.offset += cursor.offset;
        add(&mut self.stats.total_samples, timestamps.len() as u64)?;
        let record_complete = repeat.remaining == 0;
        self.repeat = (!record_complete).then_some(repeat);
        Ok(Batch {
            sample,
            timestamps,
            is_repeat: true,
            record_complete,
            record_samples: repeat.total,
        })
    }
}

fn validate_sample_count(previous: u64, additional: u64) -> Result<()> {
    if additional > (isize::MAX as u64).saturating_sub(previous) {
        Err(Error::Overflow("Sample count exceeds Py_ssize_t maximum"))
    } else {
        Ok(())
    }
}

fn decode_stack(thread: &mut ReaderThread, encoding: u8, cursor: &mut Cursor<'_>) -> Result<()> {
    let (keep, added, source) = match encoding {
        FULL => {
            let depth = cursor.var_u32()?;
            if depth > STACK_LIMIT as u32 {
                return Err(invalid(format!(
                    "Stack depth {depth} exceeds capacity {STACK_LIMIT}"
                )));
            }
            (0, depth as usize, 0)
        }
        SUFFIX => {
            let shared = cursor.var_u32()?;
            let new = cursor.var_u32()?;
            if shared as u64 > thread.depth as u64 {
                return Err(invalid(format!(
                    "Shared count {shared} exceeds current stack depth {}",
                    thread.depth
                )));
            }
            let depth = u64::from(shared) + u64::from(new);
            validate_depth(depth)?;
            (
                shared as usize,
                new as usize,
                thread.depth - shared as usize,
            )
        }
        _ => {
            let pop = cursor.var_u32()?;
            let push = cursor.var_u32()?;
            let keep = (thread.depth as u64).saturating_sub(u64::from(pop));
            validate_depth(keep + u64::from(push))?;
            (keep as usize, push as usize, pop as usize)
        }
    };
    if keep != 0 {
        thread.stack.copy_within(source..source + keep, added);
    }
    for slot in &mut thread.stack[..added] {
        *slot = cursor.var_u32()?;
    }
    thread.depth = keep + added;
    Ok(())
}

fn validate_depth(depth: u64) -> Result<()> {
    if depth > STACK_LIMIT as u64 {
        Err(invalid(format!(
            "Final stack depth {depth} exceeds capacity {STACK_LIMIT}"
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(samples: &[u8], count: u64) -> Vec<u8> {
        let mut bytes = vec![0; HEADER_SIZE];
        bytes[..4].copy_from_slice(&0x54414348u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
        bytes[28..36].copy_from_slice(&count.to_le_bytes());
        bytes[36..40].copy_from_slice(&1u32.to_le_bytes());
        let table = (HEADER_SIZE + samples.len()) as u64;
        bytes[40..48].copy_from_slice(&table.to_le_bytes());
        bytes[48..56].copy_from_slice(&table.to_le_bytes());
        bytes.extend_from_slice(samples);
        bytes.extend_from_slice(&[0; FOOTER_SIZE]);
        bytes
    }

    fn sample_header(encoding: u8) -> Vec<u8> {
        let mut bytes = vec![0; 13];
        bytes[12] = encoding;
        bytes
    }

    #[test]
    fn varint_rejects_overflow_and_truncation() {
        let mut encoded = Vec::new();
        push_varint(&mut encoded, u64::MAX).unwrap();
        let mut cursor = Cursor::new(&encoded);
        assert_eq!(cursor.varint().unwrap(), u64::MAX);
        encoded[9] = 2;
        let mut cursor = Cursor::new(&encoded);
        assert!(cursor.varint().is_err());
        let mut cursor = Cursor::new(&[128]);
        assert!(cursor.varint().is_err());
    }

    #[test]
    fn full_stack_depth_is_bounded_before_allocation() {
        let mut bytes = sample_header(1);
        bytes.extend_from_slice(&[0, 0]);
        push_varint(&mut bytes, 257).unwrap();
        let reader = Arc::new(Reader::parse(file(&bytes, 1)).unwrap());
        assert!(
            matches!(reader.replay().next_batch(), Err(Error::Value(message)) if message == "Stack depth 257 exceeds capacity 256")
        );
    }

    #[test]
    fn repeat_count_is_bounded_by_remaining_bytes() {
        let mut bytes = sample_header(0);
        push_varint(&mut bytes, u32::MAX as u64).unwrap();
        let reader = Arc::new(Reader::parse(file(&bytes, 1)).unwrap());
        assert!(
            matches!(reader.replay().next_batch(), Err(Error::Value(message)) if message == "Invalid RLE count 4294967295 exceeds maximum possible 0")
        );
    }

    #[test]
    fn empty_repeat_records_do_not_recurse() {
        let mut bytes = sample_header(0);
        bytes.push(0);
        let reader = Arc::new(Reader::parse(file(&bytes.repeat(10_000), 0)).unwrap());
        let mut replay = reader.replay();
        assert!(replay.next_batch().unwrap().is_none());
        assert_eq!(replay.stats.repeat_records, 10_000);
        assert_eq!(replay.stats.total_samples, 0);
    }

    #[test]
    fn table_count_is_bounded_before_allocation() {
        let mut bytes = file(&[], 0);
        bytes[HEADER_SIZE..HEADER_SIZE + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            matches!(Reader::parse(bytes), Err(Error::Value(message)) if message == "Invalid string count 4294967295 exceeds maximum possible 32")
        );
    }
}
