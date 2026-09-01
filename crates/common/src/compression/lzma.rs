// spell-checker:ignore ARMTHUMB chunker memlimit

//! VM-independent liblzma stream engine.
//!
//! `_lzma` is not built on Android or WebAssembly in RustPython, so the xz
//! dependency and this module have the same target boundary.  Python object
//! conversion and exception construction remain in `rustpython-stdlib`.

use xz::stream::{
    Action, Check, Error as XzError, Filters, LzmaOptions, MatchFinder, Mode, Status, Stream,
    TELL_ANY_CHECK, TELL_NO_CHECK,
};

use super::{CHUNKSIZE, Chunker};

pub const BUFSIZ: usize = 8192;
const DEF_BUF_SIZE: usize = 16 * 1024;
const USE_AFTER_FINISH_ERR: &str = "Error -2: inconsistent stream state";
const LZMA_FILTERS_MAX: usize = 4;

pub const CHECK_NONE: i32 = xz_sys::LZMA_CHECK_NONE as _;
pub const CHECK_CRC32: i32 = xz_sys::LZMA_CHECK_CRC32 as _;
pub const CHECK_CRC64: i32 = xz_sys::LZMA_CHECK_CRC64 as _;
pub const CHECK_SHA256: i32 = xz_sys::LZMA_CHECK_SHA256 as _;
pub const CHECK_ID_MAX: i32 = 15;
pub const CHECK_UNKNOWN: i32 = CHECK_ID_MAX + 1;

pub const MF_HC3: i32 = xz_sys::LZMA_MF_HC3 as _;
pub const MF_HC4: i32 = xz_sys::LZMA_MF_HC4 as _;
pub const MF_BT2: i32 = xz_sys::LZMA_MF_BT2 as _;
pub const MF_BT3: i32 = xz_sys::LZMA_MF_BT3 as _;
pub const MF_BT4: i32 = xz_sys::LZMA_MF_BT4 as _;

pub const MODE_FAST: i32 = xz_sys::LZMA_MODE_FAST as _;
pub const MODE_NORMAL: i32 = xz_sys::LZMA_MODE_NORMAL as _;

pub const FORMAT_AUTO: i32 = 0;
pub const FORMAT_XZ: i32 = 1;
pub const FORMAT_ALONE: i32 = 2;
pub const FORMAT_RAW: i32 = 3;

pub const FILTER_LZMA1: u64 = xz_sys::LZMA_FILTER_LZMA1;
pub const FILTER_LZMA2: u64 = xz_sys::LZMA_FILTER_LZMA2;
pub const FILTER_DELTA: u64 = xz_sys::LZMA_FILTER_DELTA;
pub const FILTER_X86: u64 = xz_sys::LZMA_FILTER_X86;
pub const FILTER_POWERPC: u64 = xz_sys::LZMA_FILTER_POWERPC;
pub const FILTER_IA64: u64 = xz_sys::LZMA_FILTER_IA64;
pub const FILTER_ARM: u64 = xz_sys::LZMA_FILTER_ARM;
pub const FILTER_ARMTHUMB: u64 = xz_sys::LZMA_FILTER_ARMTHUMB;
pub const FILTER_SPARC: u64 = xz_sys::LZMA_FILTER_SPARC;

pub const PRESET_DEFAULT: u32 = xz_sys::LZMA_PRESET_DEFAULT;
pub const PRESET_EXTREME: u32 = xz_sys::LZMA_PRESET_EXTREME;

const DEFAULT_LC: u32 = xz_sys::LZMA_LC_DEFAULT;
const DEFAULT_LP: u32 = xz_sys::LZMA_LP_DEFAULT;
const DEFAULT_PB: u32 = xz_sys::LZMA_PB_DEFAULT;
const DICT_POW2: [u8; 10] = [18, 20, 21, 22, 22, 23, 23, 24, 25, 26];

#[derive(Debug)]
pub enum Error {
    Memory,
    Value(String),
    Lzma(String),
    Eof,
}

impl From<XzError> for Error {
    fn from(err: XzError) -> Self {
        match err {
            XzError::UnsupportedCheck => Self::Lzma("Unsupported integrity check".to_owned()),
            XzError::Mem => Self::Memory,
            XzError::MemLimit => Self::Lzma("Memory usage limit exceeded".to_owned()),
            XzError::Format => Self::Lzma("Input format not supported by decoder".to_owned()),
            XzError::Options => Self::Lzma("Invalid or unsupported options".to_owned()),
            XzError::Data | XzError::NoCheck => Self::Lzma("Corrupt input data".to_owned()),
            XzError::Program => Self::Lzma("Internal error".to_owned()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FilterSpec {
    pub id: u64,
    pub preset: Option<u32>,
    pub dict_size: Option<u32>,
    pub lc: Option<u32>,
    pub lp: Option<u32>,
    pub pb: Option<u32>,
    pub mode: Option<u32>,
    pub nice_len: Option<u32>,
    pub mf: Option<u32>,
    pub depth: Option<u32>,
    pub dist: Option<u32>,
    pub start_offset: Option<u32>,
}

fn int_to_check(check: i32) -> Option<Check> {
    if check == -1 {
        return Some(Check::Crc64);
    }
    match check {
        CHECK_NONE => Some(Check::None),
        CHECK_CRC32 => Some(Check::Crc32),
        CHECK_CRC64 => Some(Check::Crc64),
        CHECK_SHA256 => Some(Check::Sha256),
        _ => None,
    }
}

fn u32_to_mode(value: u32) -> Option<Mode> {
    match value as i32 {
        MODE_FAST => Some(Mode::Fast),
        MODE_NORMAL => Some(Mode::Normal),
        _ => None,
    }
}

fn u32_to_mf(value: u32) -> Option<MatchFinder> {
    match value as i32 {
        MF_HC3 => Some(MatchFinder::HashChain3),
        MF_HC4 => Some(MatchFinder::HashChain4),
        MF_BT2 => Some(MatchFinder::BinaryTree2),
        MF_BT3 => Some(MatchFinder::BinaryTree3),
        MF_BT4 => Some(MatchFinder::BinaryTree4),
        _ => None,
    }
}

fn lzma_options(spec: &FilterSpec) -> Result<LzmaOptions, Error> {
    let preset = spec.preset.unwrap_or(PRESET_DEFAULT);
    let mut options = LzmaOptions::new_preset(preset)
        .map_err(|_| Error::Lzma(format!("Invalid compression preset: {preset}")))?;
    if let Some(value) = spec.dict_size {
        options.dict_size(value);
    }
    if let Some(value) = spec.lc {
        options.literal_context_bits(value);
    }
    if let Some(value) = spec.lp {
        options.literal_position_bits(value);
    }
    if let Some(value) = spec.pb {
        options.position_bits(value);
    }
    if let Some(value) = spec.mode {
        let mode = u32_to_mode(value)
            .ok_or_else(|| Error::Value("Invalid filter specifier for LZMA filter".to_owned()))?;
        options.mode(mode);
    }
    if let Some(value) = spec.nice_len {
        options.nice_len(value);
    }
    if let Some(value) = spec.mf {
        let mf = u32_to_mf(value)
            .ok_or_else(|| Error::Value("Invalid filter specifier for LZMA filter".to_owned()))?;
        options.match_finder(mf);
    }
    if let Some(value) = spec.depth {
        options.depth(value);
    }
    Ok(options)
}

fn add_bcj_filter(filters: &mut Filters, id: u64, start_offset: u32) -> Result<(), Error> {
    if start_offset == 0 {
        match id {
            FILTER_X86 => filters.x86(),
            FILTER_POWERPC => filters.powerpc(),
            FILTER_IA64 => filters.ia64(),
            FILTER_ARM => filters.arm(),
            FILTER_ARMTHUMB => filters.arm_thumb(),
            FILTER_SPARC => filters.sparc(),
            _ => unreachable!(),
        };
    } else {
        let properties = start_offset.to_le_bytes();
        match id {
            FILTER_X86 => filters.x86_properties(&properties)?,
            FILTER_POWERPC => filters.powerpc_properties(&properties)?,
            FILTER_IA64 => filters.ia64_properties(&properties)?,
            FILTER_ARM => filters.arm_properties(&properties)?,
            FILTER_ARMTHUMB => filters.arm_thumb_properties(&properties)?,
            FILTER_SPARC => filters.sparc_properties(&properties)?,
            _ => unreachable!(),
        };
    }
    Ok(())
}

fn build_filters(specs: &[FilterSpec]) -> Result<Filters, Error> {
    if specs.len() > LZMA_FILTERS_MAX {
        return Err(Error::Lzma(format!(
            "Too many filters - liblzma supports a maximum of {LZMA_FILTERS_MAX}"
        )));
    }
    let mut filters = Filters::new();
    for spec in specs {
        match spec.id {
            FILTER_LZMA1 => {
                filters.lzma1(&lzma_options(spec)?);
            }
            FILTER_LZMA2 => {
                filters.lzma2(&lzma_options(spec)?);
            }
            FILTER_DELTA => {
                let dist = spec.dist.unwrap_or(1);
                if !(1..=256).contains(&dist) {
                    return Err(Error::Value(
                        "Invalid filter specifier for delta filter".to_owned(),
                    ));
                }
                filters.delta_properties(&[(dist - 1) as u8])?;
            }
            FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB
            | FILTER_SPARC => {
                add_bcj_filter(&mut filters, spec.id, spec.start_offset.unwrap_or(0))?;
            }
            id => return Err(Error::Value(format!("Invalid filter ID: {id}"))),
        }
    }
    Ok(filters)
}

fn preset_dict_size(preset: u32) -> u32 {
    let level = (preset & xz_sys::LZMA_PRESET_LEVEL_MASK) as usize;
    DICT_POW2.get(level).map_or(0, |power| 1u32 << power)
}

fn lzma2_dict_size_from_prop(prop: u8) -> u32 {
    if prop >= 40 {
        return u32::MAX;
    }
    let prop = u32::from(prop);
    (2 | (prop & 1)) << (prop / 2 + 11)
}

fn lzma2_prop_from_dict_size(dict_size: u32) -> u8 {
    if dict_size == u32::MAX {
        return 40;
    }
    (0u8..40)
        .find(|&property| lzma2_dict_size_from_prop(property) >= dict_size)
        .unwrap_or(40)
}

pub fn encode_filter_properties(spec: &FilterSpec) -> Result<Vec<u8>, Error> {
    match spec.id {
        FILTER_LZMA1 => {
            let preset = spec.preset.unwrap_or(PRESET_DEFAULT);
            let lc = spec.lc.unwrap_or(DEFAULT_LC);
            let lp = spec.lp.unwrap_or(DEFAULT_LP);
            let pb = spec.pb.unwrap_or(DEFAULT_PB);
            if lc > 4 || lp > 4 || lc + lp > 4 || pb > 4 {
                return Err(Error::Lzma("Invalid or unsupported options".to_owned()));
            }
            let dict_size = spec.dict_size.unwrap_or_else(|| preset_dict_size(preset));
            let mut result = vec![0u8; 5];
            result[0] = ((pb * 5 + lp) * 9 + lc) as u8;
            result[1..].copy_from_slice(&dict_size.to_le_bytes());
            Ok(result)
        }
        FILTER_LZMA2 => {
            let preset = spec.preset.unwrap_or(PRESET_DEFAULT);
            let dict_size = spec.dict_size.unwrap_or_else(|| preset_dict_size(preset));
            Ok(vec![lzma2_prop_from_dict_size(dict_size)])
        }
        FILTER_DELTA => {
            let dist = spec.dist.unwrap_or(1);
            if !(1..=256).contains(&dist) {
                return Err(Error::Value(
                    "Invalid filter specifier for delta filter".to_owned(),
                ));
            }
            Ok(vec![(dist - 1) as u8])
        }
        FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB | FILTER_SPARC => {
            let start_offset = spec.start_offset.unwrap_or(0);
            Ok(if start_offset == 0 {
                vec![]
            } else {
                start_offset.to_le_bytes().to_vec()
            })
        }
        id => Err(Error::Value(format!("Invalid filter ID: {id}"))),
    }
}

pub fn decode_filter_properties(id: u64, properties: &[u8]) -> Result<FilterSpec, Error> {
    let mut spec = FilterSpec {
        id,
        ..FilterSpec::default()
    };
    match id {
        FILTER_LZMA1 => {
            let [property, a, b, c, d, ..] = properties else {
                return Err(Error::Lzma("Invalid or unsupported options".to_owned()));
            };
            let mut value = u32::from(*property);
            spec.lc = Some(value % 9);
            value /= 9;
            spec.lp = Some(value % 5);
            spec.pb = Some(value / 5);
            spec.dict_size = Some(u32::from_le_bytes([*a, *b, *c, *d]));
        }
        FILTER_LZMA2 => {
            let [property] = properties else {
                return Err(Error::Lzma("Invalid or unsupported options".to_owned()));
            };
            spec.dict_size = Some(lzma2_dict_size_from_prop(*property));
        }
        FILTER_DELTA => {
            let [property] = properties else {
                return Err(Error::Lzma("Invalid or unsupported options".to_owned()));
            };
            spec.dist = Some(u32::from(*property) + 1);
        }
        FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB | FILTER_SPARC => {
            match properties {
                [] => {}
                [a, b, c, d] => spec.start_offset = Some(u32::from_le_bytes([*a, *b, *c, *d])),
                _ => return Err(Error::Lzma("Invalid or unsupported options".to_owned())),
            }
        }
        _ => return Err(Error::Value(format!("Invalid filter ID: {id}"))),
    }
    Ok(spec)
}

#[must_use]
pub fn is_check_supported(check_id: i32) -> bool {
    unsafe { xz_sys::lzma_check_is_supported(check_id as _) != 0 }
}

struct LzmaStream {
    stream: Stream,
    check: i32,
    header_buf: [u8; 8],
    header_collected: u8,
    track_header: bool,
}

impl LzmaStream {
    fn new(stream: Stream, check: i32, track_header: bool) -> Self {
        Self {
            stream,
            check,
            header_buf: [0; 8],
            header_collected: 0,
            track_header,
        }
    }

    fn process(&mut self, input: &[u8], output: &mut Vec<u8>) -> Result<Status, XzError> {
        if self.track_header && self.header_collected < 8 {
            let count = (8 - usize::from(self.header_collected)).min(input.len());
            let start = usize::from(self.header_collected);
            self.header_buf[start..start + count].copy_from_slice(&input[..count]);
            self.header_collected += count as u8;
        }
        match self.stream.process_vec(input, output, Action::Run) {
            Ok(Status::GetCheck) => {
                if self.header_collected >= 8 {
                    self.check = i32::from(self.header_buf[7] & 0x0f);
                }
                Ok(Status::Ok)
            }
            Err(XzError::NoCheck) => {
                self.check = CHECK_NONE;
                Ok(Status::Ok)
            }
            other => other,
        }
    }
}

fn decompress_chunks(
    chunks: &mut Chunker<'_>,
    stream: &mut LzmaStream,
    max_length: Option<usize>,
) -> Result<(Vec<u8>, bool), XzError> {
    if chunks.is_empty() {
        return Ok((Vec::new(), true));
    }
    let max_length = max_length.unwrap_or(usize::MAX);
    let mut output = Vec::new();
    'outer: loop {
        let chunk = chunks.chunk();
        loop {
            let additional = BUFSIZ.min(max_length - output.capacity());
            if additional == 0 {
                return Ok((output, false));
            }
            output.reserve_exact(additional);
            let previous_in = stream.stream.total_in();
            let result = stream.process(chunk, &mut output);
            let consumed = (stream.stream.total_in() - previous_in) as usize;
            chunks.advance(consumed);
            let status = result?;
            if status == Status::StreamEnd || chunks.is_empty() {
                output.shrink_to_fit();
                return Ok((output, status == Status::StreamEnd));
            }
            if !chunk.is_empty() && consumed == 0 {
                continue;
            }
            continue 'outer;
        }
    }
}

pub struct Decompressor {
    stream: LzmaStream,
    unused_data: Vec<u8>,
    input_buffer: Vec<u8>,
    eof: bool,
    needs_input: bool,
}

impl Decompressor {
    pub fn new(
        format: i32,
        memlimit: Option<u64>,
        filters: Option<Vec<FilterSpec>>,
    ) -> Result<Self, Error> {
        if format == FORMAT_RAW && memlimit.is_some() {
            return Err(Error::Value(
                "Cannot specify memory limit with FORMAT_RAW".to_owned(),
            ));
        }
        if format == FORMAT_RAW && filters.is_none() {
            return Err(Error::Value(
                "Must specify filters for FORMAT_RAW".to_owned(),
            ));
        }
        if format != FORMAT_RAW && filters.is_some() {
            return Err(Error::Value(
                "Cannot specify filters except with FORMAT_RAW".to_owned(),
            ));
        }
        let memlimit = memlimit.unwrap_or(u64::MAX);
        let flags = TELL_ANY_CHECK | TELL_NO_CHECK;
        let stream = match format {
            FORMAT_AUTO => LzmaStream::new(
                Stream::new_auto_decoder(memlimit, flags)?,
                CHECK_UNKNOWN,
                true,
            ),
            FORMAT_XZ => LzmaStream::new(
                Stream::new_stream_decoder(memlimit, flags)?,
                CHECK_UNKNOWN,
                true,
            ),
            FORMAT_ALONE => LzmaStream::new(Stream::new_lzma_decoder(memlimit)?, CHECK_NONE, false),
            FORMAT_RAW => {
                let filters = build_filters(filters.as_deref().expect("validated raw filters"))?;
                LzmaStream::new(Stream::new_raw_decoder(&filters)?, CHECK_NONE, false)
            }
            _ => return Err(Error::Value(format!("Invalid container format: {format}"))),
        };
        Ok(Self {
            stream,
            unused_data: Vec::new(),
            input_buffer: Vec::new(),
            eof: false,
            needs_input: true,
        })
    }

    pub fn decompress(&mut self, data: &[u8], max_length: Option<usize>) -> Result<Vec<u8>, Error> {
        if self.eof {
            return Err(Error::Eof);
        }
        let input_buffer = &mut self.input_buffer;
        let stream = &mut self.stream;
        let mut chunks = Chunker::chain(input_buffer, data);
        let previous_len = chunks.len();
        let result = decompress_chunks(&mut chunks, stream, max_length);
        let stream_end = match &result {
            Ok((_, stream_end)) => *stream_end,
            Err(_) => false,
        };
        let consumed = previous_len - chunks.len();
        self.eof |= stream_end;
        if self.eof {
            self.needs_input = false;
            if !chunks.is_empty() {
                self.unused_data = chunks.to_vec();
            }
        } else if chunks.is_empty() {
            input_buffer.clear();
            self.needs_input = true;
        } else {
            self.needs_input = false;
            if let Some(consumed_from_data) = consumed.checked_sub(input_buffer.len()) {
                input_buffer.clear();
                input_buffer.extend_from_slice(&data[consumed_from_data..]);
            } else {
                input_buffer.drain(..consumed);
                input_buffer.extend_from_slice(data);
            }
        }
        result.map(|(output, _)| output).map_err(Error::from)
    }

    #[must_use]
    pub fn check(&self) -> i32 {
        self.stream.check
    }
    #[must_use]
    pub fn eof(&self) -> bool {
        self.eof
    }
    #[must_use]
    pub fn unused_data(&self) -> &[u8] {
        &self.unused_data
    }
    #[must_use]
    pub fn needs_input(&self) -> bool {
        self.needs_input
    }
}

pub struct Compressor {
    stream: Option<Stream>,
}

impl Compressor {
    pub fn new(
        format: i32,
        check: i32,
        preset: u32,
        filters: Option<Vec<FilterSpec>>,
    ) -> Result<Self, Error> {
        if format != FORMAT_XZ && check != -1 && check != CHECK_NONE {
            return Err(Error::Lzma(
                "Integrity checks are only supported by FORMAT_XZ".to_owned(),
            ));
        }
        let stream = match format {
            FORMAT_XZ => {
                let check = int_to_check(check)
                    .ok_or_else(|| Error::Value("Invalid check value".to_owned()))?;
                if let Some(specs) = filters {
                    Stream::new_stream_encoder(&build_filters(&specs)?, check)?
                } else {
                    Stream::new_easy_encoder(preset, check)?
                }
            }
            FORMAT_ALONE => {
                let options = match filters {
                    None => LzmaOptions::new_preset(preset).map_err(|_| {
                        Error::Lzma(format!("Invalid compression preset: {preset}"))
                    })?,
                    Some(specs) => match specs.as_slice() {
                        [spec] if spec.id == FILTER_LZMA1 => lzma_options(spec)?,
                        _ => {
                            return Err(Error::Value(
                                "Invalid filter chain for FORMAT_ALONE - must be a single LZMA1 filter"
                                    .to_owned(),
                            ));
                        }
                    },
                };
                Stream::new_lzma_encoder(&options)?
            }
            FORMAT_RAW => {
                let specs = filters.ok_or_else(|| {
                    Error::Value("Must specify filters for FORMAT_RAW".to_owned())
                })?;
                Stream::new_raw_encoder(&build_filters(&specs)?)?
            }
            _ => return Err(Error::Value(format!("Invalid container format: {format}"))),
        };
        Ok(Self {
            stream: Some(stream),
        })
    }

    pub fn compress(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| Error::Lzma(USE_AFTER_FINISH_ERR.to_owned()))?;
        let mut output = Vec::new();
        for mut chunk in data.chunks(CHUNKSIZE) {
            while !chunk.is_empty() {
                output.reserve(DEF_BUF_SIZE);
                let previous_in = stream.total_in();
                stream.process_vec(chunk, &mut output, Action::Run)?;
                let consumed = (stream.total_in() - previous_in) as usize;
                chunk = &chunk[consumed..];
            }
        }
        output.shrink_to_fit();
        Ok(output)
    }

    pub fn flush(&mut self) -> Result<Vec<u8>, Error> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| Error::Lzma(USE_AFTER_FINISH_ERR.to_owned()))?;
        let mut output = Vec::new();
        let status = loop {
            if output.len() == output.capacity() {
                output.reserve(DEF_BUF_SIZE);
            }
            let status = stream.process_vec(&[], &mut output, Action::Finish)?;
            if output.len() != output.capacity() {
                break status;
            }
        };
        if status == Status::StreamEnd {
            self.stream = None;
        }
        output.shrink_to_fit();
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_lzma1_properties_rejects_invalid_lclppb() {
        for (lc, lp, pb) in [(5, 0, 0), (0, 5, 0), (4, 1, 0), (0, 0, 5)] {
            let spec = FilterSpec {
                id: FILTER_LZMA1,
                lc: Some(lc),
                lp: Some(lp),
                pb: Some(pb),
                ..FilterSpec::default()
            };
            assert!(matches!(
                encode_filter_properties(&spec),
                Err(Error::Lzma(message)) if message == "Invalid or unsupported options"
            ));
        }
    }

    #[test]
    fn format_alone_uses_the_single_lzma1_filter() {
        let dict_size = 1 << 20;
        let spec = FilterSpec {
            id: FILTER_LZMA1,
            dict_size: Some(dict_size),
            ..FilterSpec::default()
        };
        let mut compressor =
            Compressor::new(FORMAT_ALONE, CHECK_NONE, PRESET_DEFAULT, Some(vec![spec])).unwrap();
        let mut encoded = compressor.compress(b"hello").unwrap();
        encoded.extend(compressor.flush().unwrap());
        assert_eq!(&encoded[1..5], &dict_size.to_le_bytes());
    }

    #[test]
    fn format_alone_rejects_other_filter_chains() {
        for filters in [
            vec![],
            vec![FilterSpec {
                id: FILTER_LZMA2,
                ..FilterSpec::default()
            }],
            vec![
                FilterSpec {
                    id: FILTER_LZMA1,
                    ..FilterSpec::default()
                },
                FilterSpec {
                    id: FILTER_LZMA1,
                    ..FilterSpec::default()
                },
            ],
        ] {
            assert!(matches!(
                Compressor::new(FORMAT_ALONE, CHECK_NONE, PRESET_DEFAULT, Some(filters)),
                Err(Error::Value(message))
                    if message
                        == "Invalid filter chain for FORMAT_ALONE - must be a single LZMA1 filter"
            ));
        }
    }
}
