// spell-checker:ignore ARMTHUMB chunker lclp memlimit

//! VM-independent liblzma stream engine.
//!
//! The engine is the `xz-core` crate, a pure-Rust port of liblzma with the
//! same entry points. Python object conversion and exception construction
//! remain in `rustpython-stdlib`.

use core::ffi::c_void;
use core::mem::MaybeUninit;

use super::Chunker;
use xz_core::common::alone_decoder::lzma_alone_decoder;
use xz_core::common::alone_encoder::lzma_alone_encoder;
use xz_core::common::auto_decoder::lzma_auto_decoder;
use xz_core::common::common::{lzma_code, lzma_end, lzma_get_check};
use xz_core::common::easy_encoder::lzma_easy_encoder;
use xz_core::common::filter_common::lzma_filters_free;
use xz_core::common::filter_decoder::{lzma_properties_decode, lzma_raw_decoder};
use xz_core::common::filter_encoder::{
    lzma_properties_encode, lzma_properties_size, lzma_raw_encoder,
};
use xz_core::common::stream_decoder::lzma_stream_decoder;
use xz_core::common::stream_encoder::lzma_stream_encoder;
use xz_core::lzma::lzma_encoder_presets::lzma_lzma_preset;
use xz_core::types::*;

const INITIAL_BUFFER_SIZE: usize = 8192;
const USE_AFTER_FINISH_ERR: &str = "Error -2: inconsistent stream state";

pub const CHECK_NONE: i32 = LZMA_CHECK_NONE as _;
pub const CHECK_CRC32: i32 = LZMA_CHECK_CRC32 as _;
pub const CHECK_CRC64: i32 = LZMA_CHECK_CRC64 as _;
pub const CHECK_SHA256: i32 = LZMA_CHECK_SHA256 as _;
pub const CHECK_ID_MAX: i32 = LZMA_CHECK_ID_MAX as _;
pub const CHECK_UNKNOWN: i32 = CHECK_ID_MAX + 1;

pub const MF_HC3: i32 = LZMA_MF_HC3 as _;
pub const MF_HC4: i32 = LZMA_MF_HC4 as _;
pub const MF_BT2: i32 = LZMA_MF_BT2 as _;
pub const MF_BT3: i32 = LZMA_MF_BT3 as _;
pub const MF_BT4: i32 = LZMA_MF_BT4 as _;

pub const MODE_FAST: i32 = LZMA_MODE_FAST as _;
pub const MODE_NORMAL: i32 = LZMA_MODE_NORMAL as _;

pub const FORMAT_AUTO: i32 = 0;
pub const FORMAT_XZ: i32 = 1;
pub const FORMAT_ALONE: i32 = 2;
pub const FORMAT_RAW: i32 = 3;

pub const FILTER_LZMA1: u64 = LZMA_FILTER_LZMA1;
pub const FILTER_LZMA2: u64 = LZMA_FILTER_LZMA2;
pub const FILTER_DELTA: u64 = LZMA_FILTER_DELTA;
pub const FILTER_X86: u64 = LZMA_FILTER_X86;
pub const FILTER_POWERPC: u64 = LZMA_FILTER_POWERPC;
pub const FILTER_IA64: u64 = LZMA_FILTER_IA64;
pub const FILTER_ARM: u64 = LZMA_FILTER_ARM;
pub const FILTER_ARMTHUMB: u64 = LZMA_FILTER_ARMTHUMB;
pub const FILTER_SPARC: u64 = LZMA_FILTER_SPARC;
pub const FILTERS_MAX: usize = 4;

pub const PRESET_DEFAULT: u32 = 6;
pub const PRESET_EXTREME: u32 = LZMA_PRESET_EXTREME;

#[derive(Debug)]
pub enum Error {
    Memory,
    Value(String),
    Lzma(String),
    Eof,
}

fn check_lzma(ret: lzma_ret) -> Result<lzma_ret, Error> {
    match ret {
        LZMA_OK | LZMA_GET_CHECK | LZMA_NO_CHECK | LZMA_STREAM_END => Ok(ret),
        LZMA_UNSUPPORTED_CHECK => Err(Error::Lzma("Unsupported integrity check".to_owned())),
        LZMA_MEM_ERROR => Err(Error::Memory),
        LZMA_MEMLIMIT_ERROR => Err(Error::Lzma("Memory usage limit exceeded".to_owned())),
        LZMA_FORMAT_ERROR => Err(Error::Lzma(
            "Input format not supported by decoder".to_owned(),
        )),
        LZMA_OPTIONS_ERROR => Err(Error::Lzma("Invalid or unsupported options".to_owned())),
        LZMA_DATA_ERROR => Err(Error::Lzma("Corrupt input data".to_owned())),
        LZMA_BUF_ERROR => Err(Error::Lzma("Insufficient buffer space".to_owned())),
        LZMA_PROG_ERROR => Err(Error::Lzma("Internal error".to_owned())),
        other => Err(Error::Lzma(format!(
            "Unrecognized error from liblzma: {other}"
        ))),
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

enum FilterOptions {
    Lzma(Box<lzma_options_lzma>),
    Delta(Box<lzma_options_delta>),
    Bcj(Box<lzma_options_bcj>),
}

struct FilterChain {
    filters: Vec<lzma_filter>,
    owned: Vec<FilterOptions>,
}

fn lzma_options(spec: &FilterSpec) -> Result<Box<lzma_options_lzma>, Error> {
    let preset = spec.preset.unwrap_or(PRESET_DEFAULT);
    let mut options = Box::new(unsafe { MaybeUninit::<lzma_options_lzma>::zeroed().assume_init() });
    if unsafe { lzma_lzma_preset(&mut *options, preset) } != 0 {
        return Err(Error::Lzma(format!("Invalid compression preset: {preset}")));
    }
    if let Some(value) = spec.dict_size {
        options.dict_size = value;
    }
    if let Some(value) = spec.lc {
        options.lc = value;
    }
    if let Some(value) = spec.lp {
        options.lp = value;
    }
    if let Some(value) = spec.pb {
        options.pb = value;
    }
    if let Some(value) = spec.mode {
        options.mode = value as lzma_mode;
    }
    if let Some(value) = spec.nice_len {
        options.nice_len = value;
    }
    if let Some(value) = spec.mf {
        options.mf = value as lzma_match_finder;
    }
    if let Some(value) = spec.depth {
        options.depth = value;
    }
    Ok(options)
}

fn filter_options(spec: &FilterSpec) -> Result<FilterOptions, Error> {
    match spec.id {
        FILTER_LZMA1 | FILTER_LZMA2 => Ok(FilterOptions::Lzma(lzma_options(spec)?)),
        FILTER_DELTA => {
            let mut options =
                Box::new(unsafe { MaybeUninit::<lzma_options_delta>::zeroed().assume_init() });
            options.type_ = LZMA_DELTA_TYPE_BYTE;
            options.dist = spec.dist.unwrap_or(1);
            Ok(FilterOptions::Delta(options))
        }
        FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB | FILTER_SPARC => {
            let mut options =
                Box::new(unsafe { MaybeUninit::<lzma_options_bcj>::zeroed().assume_init() });
            options.start_offset = spec.start_offset.unwrap_or(0);
            Ok(FilterOptions::Bcj(options))
        }
        id => Err(Error::Value(format!("Invalid filter ID: {id}"))),
    }
}

impl FilterChain {
    fn new(specs: &[FilterSpec]) -> Result<Self, Error> {
        if specs.len() > FILTERS_MAX {
            return Err(Error::Lzma(format!(
                "Too many filters - liblzma supports a maximum of {FILTERS_MAX}"
            )));
        }
        let mut chain = Self {
            filters: Vec::with_capacity(specs.len() + 1),
            owned: Vec::with_capacity(specs.len()),
        };
        for spec in specs {
            let options = filter_options(spec)?;
            let pointer = match &options {
                FilterOptions::Lzma(o) => &raw const **o as *mut c_void,
                FilterOptions::Delta(o) => &raw const **o as *mut c_void,
                FilterOptions::Bcj(o) => &raw const **o as *mut c_void,
            };
            chain.owned.push(options);
            chain.filters.push(lzma_filter {
                id: spec.id,
                options: pointer,
            });
        }
        chain.filters.push(lzma_filter {
            id: LZMA_VLI_UNKNOWN,
            options: core::ptr::null_mut(),
        });
        Ok(chain)
    }

    fn as_ptr(&self) -> *const lzma_filter {
        self.filters.as_ptr()
    }

    fn lone_lzma1_options(&self) -> Option<*const lzma_options_lzma> {
        match (self.filters.len(), self.owned.first()) {
            (2, Some(FilterOptions::Lzma(options))) if self.filters[0].id == FILTER_LZMA1 => {
                Some(&raw const **options)
            }
            _ => None,
        }
    }
}

struct Stream {
    raw: lzma_stream,
}

// The raw pointers belong to this stream and are never accessed without the
// mutable owner. Moving the owner between threads does not share the stream.
unsafe impl Send for Stream {}

impl Stream {
    fn new() -> Self {
        Self {
            raw: unsafe { MaybeUninit::<lzma_stream>::zeroed().assume_init() },
        }
    }

    fn code(
        &mut self,
        input: &[u8],
        consumed: &mut usize,
        block: &mut [u8],
        action: lzma_action,
    ) -> (lzma_ret, usize) {
        let tail = &input[*consumed..];
        self.raw.next_in = tail.as_ptr();
        self.raw.avail_in = tail.len();
        self.raw.next_out = block.as_mut_ptr();
        self.raw.avail_out = block.len();
        let ret = unsafe { lzma_code(&mut self.raw, action) };
        *consumed = input.len() - self.raw.avail_in;
        let produced = block.len() - self.raw.avail_out;
        self.raw.next_in = core::ptr::null();
        self.raw.avail_in = 0;
        self.raw.next_out = core::ptr::null_mut();
        self.raw.avail_out = 0;
        (ret, produced)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { lzma_end(&mut self.raw) };
    }
}

fn next_block(produced_so_far: usize, max_length: usize) -> Vec<u8> {
    vec![0u8; INITIAL_BUFFER_SIZE.min(max_length - produced_so_far)]
}

fn decompress_buf(
    stream: &mut Stream,
    check: &mut i32,
    eof: &mut bool,
    chunks: &mut Chunker<'_>,
    max_length: Option<usize>,
) -> Result<(Vec<u8>, bool), Error> {
    let max_length = max_length.unwrap_or(usize::MAX);
    let mut out = Vec::new();
    let mut block = next_block(0, max_length);
    let mut capped = false;
    loop {
        let chunk = chunks.chunk();
        let mut consumed = 0usize;
        let (ret, produced) = stream.code(chunk, &mut consumed, &mut block, LZMA_RUN);
        chunks.advance(consumed);
        out.extend_from_slice(&block[..produced]);
        let filled = produced == block.len();
        // BUF_ERROR after the current slice is exhausted is "need more
        // input", including when the next chained slice still has bytes.
        let ret = if ret == LZMA_BUF_ERROR && consumed == chunk.len() && !filled {
            LZMA_OK
        } else {
            ret
        };
        check_lzma(ret)?;
        if ret == LZMA_GET_CHECK || ret == LZMA_NO_CHECK {
            *check = lzma_get_check(&stream.raw) as i32;
        }
        if ret == LZMA_STREAM_END {
            *eof = true;
            break;
        }
        if filled {
            if out.len() == max_length {
                capped = true;
                break;
            }
            block = next_block(out.len(), max_length);
        } else if chunks.is_empty() {
            break;
        }
    }
    out.shrink_to_fit();
    Ok((out, capped))
}

fn int_to_check(check: i32) -> Option<lzma_check> {
    if check == -1 {
        return Some(LZMA_CHECK_CRC64);
    }
    match check {
        CHECK_NONE => Some(LZMA_CHECK_NONE),
        CHECK_CRC32 => Some(LZMA_CHECK_CRC32),
        CHECK_CRC64 => Some(LZMA_CHECK_CRC64),
        CHECK_SHA256 => Some(LZMA_CHECK_SHA256),
        _ => None,
    }
}

pub fn encode_filter_properties(spec: &FilterSpec) -> Result<Vec<u8>, Error> {
    if spec.id == FILTER_LZMA1 {
        let lc = spec.lc.unwrap_or(3);
        let lp = spec.lp.unwrap_or(0);
        let pb = spec.pb.unwrap_or(2);
        if lc > LZMA_LCLP_MAX || lp > LZMA_LCLP_MAX || lc + lp > LZMA_LCLP_MAX || pb > LZMA_PB_MAX {
            return Err(Error::Lzma("Invalid or unsupported options".to_owned()));
        }
    }
    let chain = FilterChain::new(core::slice::from_ref(spec))?;
    let filter = chain.filters[0];
    let mut encoded_size: u32 = 0;
    check_lzma(unsafe { lzma_properties_size(&mut encoded_size, &filter) })?;
    let mut properties = vec![0u8; encoded_size as usize];
    check_lzma(unsafe { lzma_properties_encode(&filter, properties.as_mut_ptr()) })?;
    Ok(properties)
}

pub fn decode_filter_properties(id: u64, properties: &[u8]) -> Result<FilterSpec, Error> {
    let mut filter = lzma_filter {
        id,
        options: core::ptr::null_mut(),
    };
    check_lzma(unsafe {
        lzma_properties_decode(
            &mut filter,
            core::ptr::null(),
            properties.as_ptr(),
            properties.len(),
        )
    })?;
    let spec = unsafe { filter_to_spec(&filter) };
    let mut chain = [
        filter,
        lzma_filter {
            id: LZMA_VLI_UNKNOWN,
            options: core::ptr::null_mut(),
        },
    ];
    unsafe { lzma_filters_free(chain.as_mut_ptr(), core::ptr::null()) };
    spec
}

/// # Safety
/// `filter.options` must be the option struct the filter's id implies, or
/// null for a filter whose properties carry nothing.
unsafe fn filter_to_spec(filter: &lzma_filter) -> Result<FilterSpec, Error> {
    let mut spec = FilterSpec {
        id: filter.id,
        ..FilterSpec::default()
    };
    match filter.id {
        FILTER_LZMA1 => {
            let options = unsafe { &*(filter.options as *const lzma_options_lzma) };
            spec.lc = Some(options.lc);
            spec.lp = Some(options.lp);
            spec.pb = Some(options.pb);
            spec.dict_size = Some(options.dict_size);
        }
        FILTER_LZMA2 => {
            let options = unsafe { &*(filter.options as *const lzma_options_lzma) };
            spec.dict_size = Some(options.dict_size);
        }
        FILTER_DELTA => {
            let options = unsafe { &*(filter.options as *const lzma_options_delta) };
            spec.dist = Some(options.dist);
        }
        FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB | FILTER_SPARC => {
            if !filter.options.is_null() {
                let options = unsafe { &*(filter.options as *const lzma_options_bcj) };
                spec.start_offset = Some(options.start_offset);
            }
        }
        id => return Err(Error::Value(format!("Invalid filter ID: {id}"))),
    }
    Ok(spec)
}

#[must_use]
pub fn is_check_supported(check_id: i32) -> bool {
    xz_core::check::check::lzma_check_is_supported(check_id as lzma_check) != 0
}

pub struct Decompressor {
    stream: Stream,
    check: i32,
    eof: bool,
    needs_input: bool,
    unused_data: Vec<u8>,
    input_buffer: Vec<u8>,
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
        const DECODER_FLAGS: u32 = LZMA_TELL_ANY_CHECK | LZMA_TELL_NO_CHECK;
        let memlimit = memlimit.unwrap_or(u64::MAX);
        let mut decompressor = Self {
            stream: Stream::new(),
            check: CHECK_UNKNOWN,
            eof: false,
            needs_input: true,
            unused_data: Vec::new(),
            input_buffer: Vec::new(),
        };
        let raw = &mut decompressor.stream.raw;
        let ret = match format {
            FORMAT_AUTO => unsafe { lzma_auto_decoder(raw, memlimit, DECODER_FLAGS) },
            FORMAT_XZ => unsafe { lzma_stream_decoder(raw, memlimit, DECODER_FLAGS) },
            FORMAT_ALONE => {
                decompressor.check = CHECK_NONE;
                unsafe { lzma_alone_decoder(raw, memlimit) }
            }
            FORMAT_RAW => {
                decompressor.check = CHECK_NONE;
                let chain = FilterChain::new(filters.as_deref().expect("validated raw filters"))?;
                unsafe { lzma_raw_decoder(raw, chain.as_ptr()) }
            }
            _ => return Err(Error::Value(format!("Invalid container format: {format}"))),
        };
        check_lzma(ret)?;
        Ok(decompressor)
    }

    pub fn decompress(&mut self, data: &[u8], max_length: Option<usize>) -> Result<Vec<u8>, Error> {
        if self.eof {
            return Err(Error::Eof);
        }
        let (out, leftover, capped) = {
            let mut chunks = Chunker::chain(&self.input_buffer, data);
            let (out, capped) = decompress_buf(
                &mut self.stream,
                &mut self.check,
                &mut self.eof,
                &mut chunks,
                max_length,
            )?;
            let leftover = if chunks.is_empty() {
                None
            } else {
                Some(chunks.to_vec())
            };
            (out, leftover, capped)
        };
        if self.eof {
            self.needs_input = false;
            self.input_buffer.clear();
            if let Some(unused) = leftover {
                self.unused_data = unused;
            }
        } else if let Some(remaining) = leftover {
            self.needs_input = false;
            self.input_buffer = remaining;
        } else {
            self.needs_input = !capped;
            self.input_buffer.clear();
        }
        Ok(out)
    }

    #[must_use]
    pub fn check(&self) -> i32 {
        self.check
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
    stream: Stream,
    flushed: bool,
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
        let mut compressor = Self {
            stream: Stream::new(),
            flushed: false,
        };
        let raw = &mut compressor.stream.raw;
        let ret = match format {
            FORMAT_XZ => {
                let check = int_to_check(check)
                    .ok_or_else(|| Error::Value("Invalid check value".to_owned()))?;
                if let Some(specs) = filters {
                    let chain = FilterChain::new(&specs)?;
                    unsafe { lzma_stream_encoder(raw, chain.as_ptr(), check) }
                } else {
                    unsafe { lzma_easy_encoder(raw, preset, check) }
                }
            }
            FORMAT_ALONE => match filters {
                None => {
                    let options = lzma_options(&FilterSpec {
                        preset: Some(preset),
                        ..FilterSpec::default()
                    })?;
                    unsafe { lzma_alone_encoder(raw, &*options) }
                }
                Some(specs) => {
                    let chain = FilterChain::new(&specs)?;
                    let Some(options) = chain.lone_lzma1_options() else {
                        return Err(Error::Value(
                            "Invalid filter chain for FORMAT_ALONE - must be a single LZMA1 filter"
                                .to_owned(),
                        ));
                    };
                    unsafe { lzma_alone_encoder(raw, options) }
                }
            },
            FORMAT_RAW => {
                let specs = filters.ok_or_else(|| {
                    Error::Value("Must specify filters for FORMAT_RAW".to_owned())
                })?;
                let chain = FilterChain::new(&specs)?;
                unsafe { lzma_raw_encoder(raw, chain.as_ptr()) }
            }
            _ => return Err(Error::Value(format!("Invalid container format: {format}"))),
        };
        check_lzma(ret)?;
        Ok(compressor)
    }

    pub fn compress(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        if self.flushed {
            return Err(Error::Lzma(USE_AFTER_FINISH_ERR.to_owned()));
        }
        self.code(data, LZMA_RUN)
    }

    pub fn flush(&mut self) -> Result<Vec<u8>, Error> {
        if self.flushed {
            return Err(Error::Lzma(USE_AFTER_FINISH_ERR.to_owned()));
        }
        self.flushed = true;
        self.code(&[], LZMA_FINISH)
    }

    fn code(&mut self, data: &[u8], action: lzma_action) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        let mut block = vec![0u8; INITIAL_BUFFER_SIZE];
        let mut consumed = 0usize;
        loop {
            let (ret, produced) = self.stream.code(data, &mut consumed, &mut block, action);
            out.extend_from_slice(&block[..produced]);
            let ret = if ret == LZMA_BUF_ERROR && data.is_empty() && produced < block.len() {
                LZMA_OK
            } else {
                ret
            };
            check_lzma(ret)?;
            if (action == LZMA_RUN && consumed == data.len())
                || (action == LZMA_FINISH && ret == LZMA_STREAM_END)
            {
                break;
            }
        }
        out.shrink_to_fit();
        Ok(out)
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
