//! The TACH v1 profiling file format. This module only reads and writes files;
//! collecting stacks from a running process is a separate capability.

use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
};

const HEADER_SIZE: usize = 64;
const FOOTER_SIZE: usize = 32;
const MAX_STACK: usize = 256;
const MAX_RLE_BYTES: usize = 16 * 1024;

#[derive(Debug)]
pub(super) enum Error {
    Io(io::Error),
    Value(String),
    Runtime(&'static str),
    Overflow(&'static str),
    Memory,
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

type Result<T> = std::result::Result<T, Error>;

fn invalid(message: impl Into<String>) -> Error {
    Error::Value(message.into())
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push((value as u8) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}

fn signed_varint(out: &mut Vec<u8>, value: i32) {
    varint(out, ((value as u32) << 1 ^ (value >> 31) as u32) as u64);
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct Frame {
    pub filename: String,
    pub funcname: String,
    pub location: [i32; 4],
    pub opcode: u8,
}

#[derive(Clone, Debug)]
struct FrameEntry {
    filename: u32,
    funcname: u32,
    location: [i32; 4],
    opcode: u8,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ProfileStats {
    pub duration: f64,
    pub sample_rate: f64,
    pub error_rate: Option<f64>,
    pub missed_samples: Option<f64>,
}

impl ProfileStats {
    pub(super) fn validate(&self) -> Result<()> {
        for (value, name) in [
            (self.duration, "duration"),
            (self.sample_rate, "sample rate"),
        ]
        .into_iter()
        .chain(self.error_rate.map(|value| (value, "error rate")))
        {
            if !value.is_finite() || value < 0.0 {
                return Err(invalid(format!(
                    "{name} must be a finite non-negative value"
                )));
            }
        }
        if self.missed_samples.is_some_and(|value| !value.is_finite()) {
            return Err(invalid("missed samples must be a finite value"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Stats {
    pub records: [u64; 4],
    pub repeat_samples: u64,
    pub total_samples: u64,
    pub total_frames_written: u64,
    pub frames_saved: u64,
    pub bytes_written: u64,
    pub stack_reconstructions: u64,
}

#[derive(Debug)]
struct WriterThread {
    id: u64,
    interpreter: u32,
    timestamp: u64,
    stack: Vec<u32>,
    repeats: Vec<u8>,
    repeat_count: u32,
}

#[derive(Debug)]
pub(super) struct Writer {
    // Present until Drop takes ownership without flushing the buffer.
    file: Option<BufWriter<File>>,
    start_time: u64,
    interval: u64,
    config: u32,
    strings: Vec<String>,
    string_indices: HashMap<String, u32>,
    frames: Vec<FrameEntry>,
    frame_indices: HashMap<Frame, u32>,
    threads: Vec<WriterThread>,
    thread_indices: HashMap<(u64, u32), usize>,
    pub stats: Stats,
    pub profile_stats: Option<ProfileStats>,
}

impl Drop for Writer {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            // Finalization flushes pending samples. Close and destruction
            // discard them instead of letting BufWriter flush on drop.
            let _ = file.into_parts();
        }
    }
}

impl Writer {
    pub(super) fn new(
        file: File,
        interval: u64,
        start_time: u64,
        mode: i32,
        features: i32,
    ) -> Result<Self> {
        let mut file = BufWriter::with_capacity(512 * 1024, file);
        file.write_all(&[0; HEADER_SIZE])?;
        // Publish the placeholder even if the writer is closed before finalizing.
        file.flush()?;
        Ok(Self {
            file: Some(file),
            start_time,
            interval,
            config: (mode + 1) as u32
                | if features < 0 {
                    0
                } else {
                    8 | (features as u32) << 4
                },
            strings: Vec::new(),
            string_indices: HashMap::new(),
            frames: Vec::new(),
            frame_indices: HashMap::new(),
            threads: Vec::new(),
            thread_indices: HashMap::new(),
            stats: Stats::default(),
            profile_stats: None,
        })
    }

    fn intern_string(&mut self, value: &str) -> Result<u32> {
        if let Some(&index) = self.string_indices.get(value) {
            return Ok(index);
        }
        u32::try_from(value.len()).map_err(|_| Error::Overflow("String is too long"))?;
        let index =
            u32::try_from(self.strings.len()).map_err(|_| Error::Overflow("Too many strings"))?;
        self.strings.push(value.to_owned());
        self.string_indices.insert(value.to_owned(), index);
        Ok(index)
    }

    fn intern_frame(&mut self, frame: Frame) -> Result<u32> {
        if let Some(&index) = self.frame_indices.get(&frame) {
            return Ok(index);
        }
        let index =
            u32::try_from(self.frames.len()).map_err(|_| Error::Overflow("Too many frames"))?;
        let entry = FrameEntry {
            filename: self.intern_string(&frame.filename)?,
            funcname: self.intern_string(&frame.funcname)?,
            location: frame.location,
            opcode: frame.opcode,
        };
        self.frames.push(entry);
        self.frame_indices.insert(frame, index);
        Ok(index)
    }

    fn header(out: &mut Vec<u8>, thread: &WriterThread, encoding: u8) {
        out.extend_from_slice(&thread.id.to_le_bytes());
        out.extend_from_slice(&thread.interpreter.to_le_bytes());
        out.push(encoding);
    }

    fn flush_repeats(&mut self, index: usize) -> Result<()> {
        let thread = &mut self.threads[index];
        if thread.repeat_count == 0 {
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(18 + thread.repeats.len());
        Self::header(&mut bytes, thread, 0);
        varint(&mut bytes, thread.repeat_count as u64);
        bytes.extend_from_slice(&thread.repeats);
        self.file.as_mut().unwrap().write_all(&bytes)?;
        self.stats.records[0] += 1;
        self.stats.repeat_samples += thread.repeat_count as u64;
        self.stats.frames_saved += thread.repeat_count as u64 * thread.stack.len() as u64;
        self.stats.bytes_written += bytes.len() as u64;
        thread.repeats.clear();
        thread.repeat_count = 0;
        Ok(())
    }

    pub(super) fn sample(
        &mut self,
        id: u64,
        interpreter: u32,
        status: u8,
        frames: Vec<Frame>,
        timestamp: u64,
    ) -> Result<()> {
        let stack = frames
            .into_iter()
            .take(MAX_STACK)
            .map(|frame| self.intern_frame(frame))
            .collect::<Result<Vec<_>>>()?;
        let key = (id, interpreter);
        let index = match self.thread_indices.get(&key) {
            Some(&index) => index,
            None => {
                u32::try_from(self.threads.len() + 1)
                    .map_err(|_| Error::Overflow("Too many threads"))?;
                let index = self.threads.len();
                self.threads.push(WriterThread {
                    id,
                    interpreter,
                    timestamp: self.start_time,
                    stack: Vec::new(),
                    repeats: Vec::new(),
                    repeat_count: 0,
                });
                self.thread_indices.insert(key, index);
                index
            }
        };
        let delta = timestamp.wrapping_sub(self.threads[index].timestamp);
        if stack == self.threads[index].stack {
            if self.threads[index].repeats.len() + 11 > MAX_RLE_BYTES {
                self.flush_repeats(index)?;
            }
            let thread = &mut self.threads[index];
            varint(&mut thread.repeats, delta);
            thread.repeats.push(status);
            thread.repeat_count += 1;
        } else {
            self.flush_repeats(index)?;
            let thread = &mut self.threads[index];
            let shared = stack
                .iter()
                .rev()
                .zip(thread.stack.iter().rev())
                .take_while(|(left, right)| left == right)
                .count();
            let pop = thread.stack.len() - shared;
            let push = stack.len() - shared;
            let encoding = if shared == 0 {
                1
            } else if pop == 0 && push > 0 {
                2
            } else if shared < stack.len() / 2 {
                1
            } else {
                3
            };
            let mut bytes = Vec::with_capacity(26 + stack.len() * 5);
            Self::header(&mut bytes, thread, encoding);
            varint(&mut bytes, delta);
            bytes.push(status);
            let written = if encoding == 1 {
                varint(&mut bytes, stack.len() as u64);
                stack.len()
            } else {
                varint(&mut bytes, if encoding == 2 { shared } else { pop } as u64);
                varint(&mut bytes, push as u64);
                push
            };
            for &frame in &stack[..written] {
                varint(&mut bytes, frame as u64);
            }
            self.file.as_mut().unwrap().write_all(&bytes)?;
            self.stats.records[encoding as usize] += 1;
            self.stats.total_frames_written += written as u64;
            self.stats.frames_saved += (stack.len() - written) as u64;
            self.stats.bytes_written += bytes.len() as u64;
            thread.stack = stack;
        }
        self.threads[index].timestamp = timestamp;
        self.stats.total_samples += 1;
        Ok(())
    }

    pub(super) fn finalize(&mut self, python_version: [u8; 3]) -> Result<()> {
        for index in 0..self.threads.len() {
            self.flush_repeats(index)?;
        }
        let file = self.file.as_mut().unwrap();
        let string_offset = file.stream_position()?;
        let mut buffer = Vec::new();
        for string in &self.strings {
            buffer.clear();
            varint(&mut buffer, string.len() as u64);
            file.write_all(&buffer)?;
            file.write_all(string.as_bytes())?;
        }
        let frame_offset = file.stream_position()?;
        for frame in &self.frames {
            buffer.clear();
            varint(&mut buffer, frame.filename as u64);
            varint(&mut buffer, frame.funcname as u64);
            for pair in frame.location.as_chunks::<2>().0 {
                signed_varint(&mut buffer, pair[0]);
                signed_varint(
                    &mut buffer,
                    if pair[0] == -1 || pair[1] == -1 {
                        0
                    } else {
                        pair[1].wrapping_sub(pair[0])
                    },
                );
            }
            buffer.push(frame.opcode);
            file.write_all(&buffer)?;
        }
        if let Some(stats) = self.profile_stats {
            buffer.clear();
            for value in [
                stats.duration,
                stats.sample_rate,
                stats.error_rate.unwrap_or_default(),
                stats.missed_samples.unwrap_or_default(),
            ] {
                buffer.extend_from_slice(&value.to_le_bytes());
            }
            let present = u32::from(stats.error_rate.is_some())
                | (u32::from(stats.missed_samples.is_some()) << 1);
            buffer.extend_from_slice(&present.to_le_bytes());
            buffer.extend_from_slice(&[0; 4]);
            buffer.extend_from_slice(b"TACHSTAT");
            buffer.extend_from_slice(&1u32.to_le_bytes());
            buffer.extend_from_slice(&56u32.to_le_bytes());
            file.write_all(&buffer)?;
        }
        let size = file.stream_position()? + FOOTER_SIZE as u64;
        buffer.clear();
        buffer.extend_from_slice(&(self.strings.len() as u32).to_le_bytes());
        buffer.extend_from_slice(&(self.frames.len() as u32).to_le_bytes());
        buffer.extend_from_slice(&size.to_le_bytes());
        buffer.extend_from_slice(&[0; 16]);
        file.write_all(&buffer)?;
        buffer.clear();
        buffer.extend_from_slice(&0x54414348u32.to_le_bytes());
        buffer.extend_from_slice(&1u32.to_le_bytes());
        buffer.extend_from_slice(&python_version);
        buffer.push(0);
        for value in [self.start_time, self.interval, self.stats.total_samples] {
            buffer.extend_from_slice(&value.to_le_bytes());
        }
        buffer.extend_from_slice(&(self.threads.len() as u32).to_le_bytes());
        buffer.extend_from_slice(&string_offset.to_le_bytes());
        buffer.extend_from_slice(&frame_offset.to_le_bytes());
        buffer.extend_from_slice(&0u32.to_le_bytes());
        buffer.extend_from_slice(&self.config.to_le_bytes());
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&buffer)?;
        file.flush()?;
        Ok(())
    }
}

#[derive(Debug)]
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    big_endian: bool,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| invalid("Invalid file offset"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("Unexpected end of file"))?;
        self.offset = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        let bytes = self.take(4)?.try_into().unwrap();
        Ok(if self.big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    }
    fn u64(&mut self) -> Result<u64> {
        let bytes = self.take(8)?.try_into().unwrap();
        Ok(if self.big_endian {
            u64::from_be_bytes(bytes)
        } else {
            u64::from_le_bytes(bytes)
        })
    }
    fn varint(&mut self) -> Result<u64> {
        let mut value = 0;
        for shift in (0..70).step_by(7) {
            let byte = self.byte().map_err(|_| invalid("Malformed varint"))?;
            if shift == 63 && byte > 1 {
                return Err(invalid("Malformed varint"));
            }
            value |= (byte as u64 & 127) << shift;
            if byte & 128 == 0 {
                return Ok(value);
            }
        }
        Err(invalid("Malformed varint"))
    }
    fn varint32(&mut self) -> Result<u32> {
        u32::try_from(self.varint()?).map_err(|_| invalid("Malformed varint"))
    }
    fn signed(&mut self) -> Result<i32> {
        let value = self.varint32()?;
        Ok((value >> 1) as i32 ^ -((value & 1) as i32))
    }
}

#[derive(Debug)]
pub(super) struct Reader {
    bytes: Vec<u8>,
    sample_end: usize,
    big_endian: bool,
    pub python_version: [u8; 3],
    pub start_time: u64,
    pub interval: u64,
    pub sample_count: u64,
    pub thread_count: u32,
    pub mode: Option<u32>,
    pub features: Option<u32>,
    pub profile_stats: Option<ProfileStats>,
    pub strings: Vec<String>,
    frames: Vec<FrameEntry>,
}

impl Reader {
    pub(super) fn open(mut file: File) -> Result<Self> {
        let size = usize::try_from(file.metadata()?.len())
            .map_err(|_| Error::Overflow("File is too large to read"))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| Error::Memory)?;
        file.read_to_end(&mut bytes)?;
        Self::parse(bytes)
    }

    pub(super) fn parse(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < HEADER_SIZE {
            return Err(invalid("File too small for header"));
        }
        let mut cursor = Cursor {
            bytes: &bytes,
            offset: 0,
            big_endian: false,
        };
        let magic = cursor.u32()?;
        cursor.big_endian = match magic {
            0x54414348 => false,
            0x48434154 => true,
            _ => return Err(invalid(format!("Invalid magic number: 0x{magic:08x}"))),
        };
        let version = cursor.u32()?;
        if version != 1 {
            return Err(invalid(format!(
                "Unsupported format version {version} (this reader supports version 1)"
            )));
        }
        let python_version = cursor.take(4)?[..3].try_into().unwrap();
        let start_time = cursor.u64()?;
        let interval = cursor.u64()?;
        let sample_count = cursor.u64()?;
        let thread_count = cursor.u32()?;
        let string_offset = cursor.u64()?;
        let frame_offset = cursor.u64()?;
        let compression = cursor.u32()?;
        let config = cursor.u32()?;
        if config & 7 > 5 {
            return Err(invalid(format!(
                "Invalid profiling mode in header: {}",
                config & 7
            )));
        }
        for (name, offset) in [("string", string_offset), ("frame", frame_offset)] {
            if offset > bytes.len() as u64 {
                return Err(invalid(format!(
                    "Invalid {name} table offset: {offset} exceeds file size {}",
                    bytes.len()
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
        match compression {
            0 => (),
            1 => {
                return Err(Error::Runtime(
                    "File uses zstd compression but zstd support not compiled in",
                ));
            }
            _ => {
                return Err(invalid(format!(
                    "Unsupported compression type: {compression}"
                )));
            }
        }
        cursor.offset = bytes.len() - FOOTER_SIZE;
        let string_count = cursor.u32()?;
        let frame_count = cursor.u32()?;
        let mut profile_stats = None;
        if bytes.len() >= FOOTER_SIZE + 32
            && &bytes[bytes.len() - 48..bytes.len() - 40] == b"TACHSTAT"
        {
            cursor.offset = bytes.len() - 40;
            let version = cursor.u32()?;
            let size = cursor.u32()? as usize;
            if size < 32 || size > bytes.len() - FOOTER_SIZE {
                return Err(invalid("Invalid profiling statistics size"));
            }
            if version == 1 {
                cursor.offset = bytes.len() - FOOTER_SIZE - size;
                let mut stats = ProfileStats {
                    duration: f64::from_bits(cursor.u64()?),
                    sample_rate: f64::from_bits(cursor.u64()?),
                    error_rate: None,
                    missed_samples: None,
                };
                if size >= 56 {
                    let error = f64::from_bits(cursor.u64()?);
                    let missed = f64::from_bits(cursor.u64()?);
                    let present = cursor.u32()?;
                    stats.error_rate = (present & 1 != 0).then_some(error);
                    stats.missed_samples = (present & 2 != 0).then_some(missed);
                }
                stats
                    .validate()
                    .map_err(|_| invalid("Invalid profiling statistics values"))?;
                profile_stats = Some(stats);
            }
        }
        let string_offset = string_offset as usize;
        let frame_offset = frame_offset as usize;
        for (name, count, offset, entry_size) in [
            ("string", string_count, string_offset, 1),
            ("frame", frame_count, frame_offset, 7),
        ] {
            let maximum = (bytes.len() - offset) / entry_size;
            if count as usize > maximum {
                return Err(invalid(format!(
                    "Invalid {name} count {count} exceeds maximum possible {maximum}"
                )));
            }
        }
        let mut strings = Vec::new();
        strings
            .try_reserve_exact(string_count as usize)
            .map_err(|_| Error::Memory)?;
        cursor.offset = string_offset;
        for _ in 0..string_count {
            let size = cursor.varint32()? as usize;
            let string = cursor
                .take(size)
                .map_err(|_| invalid("String table overflow"))?;
            strings.push(String::from_utf8_lossy(string).into_owned());
        }
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(frame_count as usize)
            .map_err(|_| Error::Memory)?;
        cursor.offset = frame_offset;
        for _ in 0..frame_count {
            let filename = cursor.varint32()?;
            let funcname = cursor.varint32()?;
            let mut location = [0; 4];
            for pair in location.as_chunks_mut::<2>().0 {
                pair[0] = cursor.signed()?;
                let delta = cursor.signed()?;
                pair[1] = if pair[0] == -1 {
                    -1
                } else {
                    pair[0].wrapping_add(delta)
                };
            }
            frames.push(FrameEntry {
                filename,
                funcname,
                location,
                opcode: cursor.byte()?,
            });
        }
        let big_endian = cursor.big_endian;
        Ok(Self {
            bytes,
            sample_end: string_offset,
            big_endian,
            python_version,
            start_time,
            interval,
            sample_count,
            thread_count,
            mode: (config & 7).checked_sub(1),
            features: (config & 8 != 0).then_some((config >> 4) & 31),
            profile_stats,
            strings,
            frames,
        })
    }

    pub(super) fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub(super) fn frame(&self, index: u32) -> Result<Frame> {
        let frame = self
            .frames
            .get(index as usize)
            .ok_or_else(|| invalid(format!("Invalid frame index: {index}")))?;
        let filename = self
            .strings
            .get(frame.filename as usize)
            .ok_or_else(|| invalid("Invalid string index in frame"))?
            .clone();
        let funcname = self
            .strings
            .get(frame.funcname as usize)
            .ok_or_else(|| invalid("Invalid string index in frame"))?
            .clone();
        Ok(Frame {
            filename,
            funcname,
            location: frame.location,
            opcode: frame.opcode,
        })
    }

    pub(super) fn replay(&self) -> Replay<'_> {
        Replay {
            reader: self,
            cursor: Cursor {
                bytes: &self.bytes[..self.sample_end],
                offset: HEADER_SIZE,
                big_endian: self.big_endian,
            },
            threads: HashMap::new(),
            stats: Stats::default(),
            pending: None,
        }
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
        varint(&mut encoded, u64::MAX);
        let mut cursor = Cursor {
            bytes: &encoded,
            offset: 0,
            big_endian: false,
        };
        assert_eq!(cursor.varint().unwrap(), u64::MAX);
        encoded[9] = 2;
        let mut cursor = Cursor {
            bytes: &encoded,
            offset: 0,
            big_endian: false,
        };
        assert!(cursor.varint().is_err());
        let mut cursor = Cursor {
            bytes: &[128],
            offset: 0,
            big_endian: false,
        };
        assert!(cursor.varint().is_err());
    }

    #[test]
    fn full_stack_depth_is_bounded_before_allocation() {
        let mut bytes = sample_header(1);
        bytes.extend_from_slice(&[0, 0]);
        varint(&mut bytes, 257);
        let reader = Reader::parse(file(&bytes, 1)).unwrap();
        assert!(
            matches!(reader.replay().next(), Err(Error::Value(message)) if message == "Stack depth 257 exceeds capacity 256")
        );
    }

    #[test]
    fn repeat_count_is_bounded_by_remaining_bytes() {
        let mut bytes = sample_header(0);
        varint(&mut bytes, u32::MAX as u64);
        let reader = Reader::parse(file(&bytes, 1)).unwrap();
        assert!(
            matches!(reader.replay().next(), Err(Error::Value(message)) if message == "Invalid RLE count 4294967295 exceeds maximum possible 0")
        );
    }

    #[test]
    fn empty_repeat_records_do_not_recurse() {
        let mut bytes = sample_header(0);
        bytes.push(0);
        let reader = Reader::parse(file(&bytes.repeat(10_000), 0)).unwrap();
        let mut replay = reader.replay();
        let mut records = 0;
        while let Some(batch) = replay.next().unwrap() {
            assert!(batch.timestamps.is_empty());
            records += 1;
        }
        assert_eq!(records, 10_000);
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

#[derive(Debug, Default)]
struct ReaderThread {
    timestamp: u64,
    stack: Vec<u32>,
}

pub(super) struct Batch {
    pub id: u64,
    pub interpreter: u32,
    pub status: u8,
    pub stack: Vec<u32>,
    pub timestamps: Vec<u64>,
}

struct Repeat {
    id: u64,
    interpreter: u32,
    remaining: u32,
}

pub(super) struct Replay<'a> {
    reader: &'a Reader,
    cursor: Cursor<'a>,
    threads: HashMap<(u64, u32), ReaderThread>,
    pub stats: Stats,
    pending: Option<Repeat>,
}

impl Replay<'_> {
    pub(super) fn next(&mut self) -> Result<Option<Batch>> {
        if let Some(mut repeat) = self.pending.take() {
            let thread = self
                .threads
                .get_mut(&(repeat.id, repeat.interpreter))
                .unwrap();
            let mut status = 0;
            let mut timestamps = Vec::new();
            // A file can advertise a large repeat group; emit bounded batches.
            while repeat.remaining != 0 && timestamps.len() < MAX_RLE_BYTES {
                let before = self.cursor.offset;
                let delta = self.cursor.varint()?;
                let current_status = self.cursor.byte()?;
                if !timestamps.is_empty() && status != current_status {
                    self.cursor.offset = before;
                    break;
                }
                status = current_status;
                thread.timestamp = thread.timestamp.wrapping_add(delta);
                timestamps.push(thread.timestamp);
                repeat.remaining -= 1;
            }
            let batch = Batch {
                id: repeat.id,
                interpreter: repeat.interpreter,
                status,
                stack: thread.stack.clone(),
                timestamps,
            };
            self.stats.total_samples = self
                .stats
                .total_samples
                .checked_add(batch.timestamps.len() as u64)
                .ok_or(Error::Overflow("Sample count exceeds maximum"))?;
            if repeat.remaining > 0 {
                self.pending = Some(repeat);
            }
            return Ok(Some(batch));
        }
        let remaining = self.cursor.bytes.len() - self.cursor.offset;
        if remaining == 0 {
            if self.stats.total_samples != self.reader.sample_count {
                return Err(invalid(format!(
                    "Sample count mismatch: header declares {} samples but replay decoded {}",
                    self.reader.sample_count, self.stats.total_samples
                )));
            }
            return Ok(None);
        }
        if remaining < 13 {
            return Err(invalid(format!(
                "Truncated sample data: {remaining} trailing bytes"
            )));
        }
        let id = self.cursor.u64()?;
        let interpreter = self.cursor.u32()?;
        let encoding = self.cursor.byte()?;
        let key = (id, interpreter);
        if !self.threads.contains_key(&key) {
            if self.threads.len() >= self.reader.thread_count as usize {
                return Err(invalid(format!(
                    "Invalid thread count: sample data contains more unique threads than declared in header (declared {}, found at least {})",
                    self.reader.thread_count,
                    self.threads.len() + 1
                )));
            }
            self.threads.insert(
                key,
                ReaderThread {
                    timestamp: self.reader.start_time,
                    stack: Vec::new(),
                },
            );
        }
        if encoding > 3 {
            return Err(invalid(format!("Unknown stack encoding: {encoding}")));
        }
        self.stats.records[encoding as usize] += 1;
        if encoding == 0 {
            let count = self.cursor.varint32()?;
            let max = (self.cursor.bytes.len() - self.cursor.offset) / 2;
            if count as usize > max {
                return Err(invalid(format!(
                    "Invalid RLE count {count} exceeds maximum possible {max}"
                )));
            }
            self.stats.repeat_samples += count as u64;
            if count != 0 {
                self.pending = Some(Repeat {
                    id,
                    interpreter,
                    remaining: count,
                });
            }
            // Continue iteratively for a zero-length repeat record, avoiding
            // recursion controlled by untrusted file contents.
            if count == 0 {
                return Ok(Some(Batch {
                    id,
                    interpreter,
                    status: 0,
                    stack: Vec::new(),
                    timestamps: Vec::new(),
                }));
            }
            return self.next();
        }
        let delta = self.cursor.varint()?;
        let status = self.cursor.byte()?;
        let thread = self.threads.get_mut(&key).unwrap();
        thread.timestamp = thread.timestamp.wrapping_add(delta);
        let first = self.cursor.varint32()? as usize;
        let (keep, push) = if encoding == 1 {
            (0, first)
        } else {
            let push = self.cursor.varint32()? as usize;
            let keep = if encoding == 2 {
                if first > thread.stack.len() {
                    return Err(invalid(format!(
                        "Shared count {first} exceeds current stack depth {}",
                        thread.stack.len()
                    )));
                }
                first
            } else {
                thread.stack.len().saturating_sub(first)
            };
            (keep, push)
        };
        let depth = keep as u64 + push as u64;
        if depth > MAX_STACK as u64 {
            let name = if encoding == 1 {
                "Stack"
            } else {
                "Final stack"
            };
            return Err(invalid(format!(
                "{name} depth {depth} exceeds capacity {MAX_STACK}"
            )));
        }
        let mut stack = Vec::with_capacity(depth as usize);
        for _ in 0..push {
            stack.push(self.cursor.varint32()?);
        }
        stack.extend_from_slice(&thread.stack[thread.stack.len() - keep..]);
        thread.stack = stack;
        self.stats.stack_reconstructions += 1;
        self.stats.total_samples += 1;
        Ok(Some(Batch {
            id,
            interpreter,
            status,
            stack: thread.stack.clone(),
            timestamps: vec![thread.timestamp],
        }))
    }
}
