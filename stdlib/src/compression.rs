// cspell:ignore chunker

//! internal shared module for compression libraries

use crate::vm::function::{ArgBytesLike, ArgSize, OptionalArg};
use crate::vm::{
    PyResult, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyBytesRef},
    convert::ToPyException,
};

pub const USE_AFTER_FINISH_ERR: &str = "Error -2: inconsistent stream state";
// TODO: don't hardcode
const CHUNKSIZE: usize = u32::MAX as usize;

#[derive(FromArgs)]
pub struct DecompressArgs {
    #[pyarg(positional)]
    data: ArgBytesLike,
    #[pyarg(any, optional)]
    max_length: OptionalArg<ArgSize>,
}

impl DecompressArgs {
    pub fn data(&self) -> crate::common::borrow::BorrowedValue<'_, [u8]> {
        self.data.borrow_buf()
    }
    pub fn raw_max_length(&self) -> Option<isize> {
        self.max_length.into_option().map(|ArgSize { value }| value)
    }

    // negative is None
    pub fn max_length(&self) -> Option<usize> {
        self.max_length
            .into_option()
            .and_then(|ArgSize { value }| usize::try_from(value).ok())
    }
}

pub trait Decompressor {
    type Flush: DecompressFlushKind;
    type Status: DecompressStatus;
    type Error;

    fn total_in(&self) -> u64;
    fn decompress_vec(
        &mut self,
        input: &[u8],
        output: &mut Vec<u8>,
        flush: Self::Flush,
    ) -> Result<Self::Status, Self::Error>;
    fn maybe_set_dict(&mut self, err: Self::Error) -> Result<(), Self::Error> {
        Err(err)
    }
}

pub trait DecompressStatus {
    fn is_stream_end(&self) -> bool;
}

pub trait DecompressFlushKind: Copy {
    const SYNC: Self;
}

impl DecompressFlushKind for () {
    const SYNC: Self = ();
}

pub fn flush_sync<T: DecompressFlushKind>(_final_chunk: bool) -> T {
    T::SYNC
}

#[derive(Clone)]
pub struct Chunker<'a> {
    data1: &'a [u8],
    data2: &'a [u8],
}
impl<'a> Chunker<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data1: data,
            data2: &[],
        }
    }
    pub fn chain(data1: &'a [u8], data2: &'a [u8]) -> Self {
        if data1.is_empty() {
            Self {
                data1: data2,
                data2: &[],
            }
        } else {
            Self { data1, data2 }
        }
    }
    pub fn len(&self) -> usize {
        self.data1.len() + self.data2.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data1.is_empty()
    }
    pub fn to_vec(&self) -> Vec<u8> {
        [self.data1, self.data2].concat()
    }
    pub fn chunk(&self) -> &'a [u8] {
        self.data1.get(..CHUNKSIZE).unwrap_or(self.data1)
    }
    pub fn advance(&mut self, consumed: usize) {
        self.data1 = &self.data1[consumed..];
        if self.data1.is_empty() {
            self.data1 = std::mem::take(&mut self.data2);
        }
    }
}

pub fn _decompress<D: Decompressor>(
    data: &[u8],
    d: &mut D,
    bufsize: usize,
    max_length: Option<usize>,
    calc_flush: impl Fn(bool) -> D::Flush,
) -> Result<(Vec<u8>, bool), D::Error> {
    let mut data = Chunker::new(data);
    _decompress_chunks(&mut data, d, bufsize, max_length, calc_flush)
}

pub fn _decompress_chunks<D: Decompressor>(
    data: &mut Chunker<'_>,
    d: &mut D,
    bufsize: usize,
    max_length: Option<usize>,
    calc_flush: impl Fn(bool) -> D::Flush,
) -> Result<(Vec<u8>, bool), D::Error> {
    if data.is_empty() {
        return Ok((Vec::new(), true));
    }
    let max_length = max_length.unwrap_or(usize::MAX);
    let mut buf = Vec::new();

    'outer: loop {
        let chunk = data.chunk();
        let flush = calc_flush(chunk.len() == data.len());
        loop {
            let additional = std::cmp::min(bufsize, max_length - buf.capacity());
            if additional == 0 {
                return Ok((buf, false));
            }
            buf.reserve_exact(additional);

            let prev_in = d.total_in();
            let res = d.decompress_vec(chunk, &mut buf, flush);
            let consumed = d.total_in() - prev_in;

            data.advance(consumed as usize);

            match res {
                Ok(status) => {
                    let stream_end = status.is_stream_end();
                    if stream_end || data.is_empty() {
                        // we've reached the end of the stream, we're done
                        buf.shrink_to_fit();
                        return Ok((buf, stream_end));
                    } else if !chunk.is_empty() && consumed == 0 {
                        // we're gonna need a bigger buffer
                        continue;
                    } else {
                        // next chunk
                        continue 'outer;
                    }
                }
                Err(e) => {
                    d.maybe_set_dict(e)?;
                    // now try the next chunk
                    continue 'outer;
                }
            };
        }
    }
}

pub trait Compressor {
    type Status: CompressStatusKind;
    type Flush: CompressFlushKind;
    const CHUNKSIZE: usize;
    const DEF_BUF_SIZE: usize;

    fn compress_vec(
        &mut self,
        input: &[u8],
        output: &mut Vec<u8>,
        flush: Self::Flush,
        vm: &VirtualMachine,
    ) -> PyResult<Self::Status>;

    fn total_in(&mut self) -> usize;

    fn new_error(message: impl Into<String>, vm: &VirtualMachine) -> PyBaseExceptionRef;
}

pub trait CompressFlushKind: Copy {
    const NONE: Self;
    const FINISH: Self;

    fn to_usize(self) -> usize;
}

pub trait CompressStatusKind: Copy {
    const OK: Self;
    const EOF: Self;

    fn to_usize(self) -> usize;
}

#[derive(Debug)]
pub struct CompressState<C: Compressor> {
    compressor: Option<C>,
}

impl<C: Compressor> CompressState<C> {
    pub fn new(compressor: C) -> Self {
        Self {
            compressor: Some(compressor),
        }
    }

    fn get_compressor(&mut self, vm: &VirtualMachine) -> PyResult<&mut C> {
        self.compressor
            .as_mut()
            .ok_or_else(|| C::new_error(USE_AFTER_FINISH_ERR, vm))
    }

    pub fn compress(&mut self, data: &[u8], vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut buf = Vec::new();
        let compressor = self.get_compressor(vm)?;

        for mut chunk in data.chunks(C::CHUNKSIZE) {
            while !chunk.is_empty() {
                buf.reserve(C::DEF_BUF_SIZE);
                let prev_in = compressor.total_in();
                compressor.compress_vec(chunk, &mut buf, C::Flush::NONE, vm)?;
                let consumed = compressor.total_in() - prev_in;
                chunk = &chunk[consumed..];
            }
        }

        buf.shrink_to_fit();
        Ok(buf)
    }

    pub fn flush(&mut self, mode: C::Flush, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut buf = Vec::new();
        let compressor = self.get_compressor(vm)?;

        let status = loop {
            if buf.len() == buf.capacity() {
                buf.reserve(C::DEF_BUF_SIZE);
            }
            let status = compressor.compress_vec(&[], &mut buf, mode, vm)?;
            if buf.len() != buf.capacity() {
                break status;
            }
        };

        if status.to_usize() == C::Status::EOF.to_usize() {
            if mode.to_usize() == C::Flush::FINISH.to_usize() {
                self.compressor = None;
            } else {
                return Err(C::new_error("unexpected eof", vm));
            }
        }

        buf.shrink_to_fit();
        Ok(buf)
    }
}

#[derive(Debug)]
pub struct DecompressState<D> {
    decompress: D,
    unused_data: PyBytesRef,
    input_buffer: Vec<u8>,
    eof: bool,
    needs_input: bool,
}

impl<D: Decompressor> DecompressState<D> {
    pub fn new(decompress: D, vm: &VirtualMachine) -> Self {
        Self {
            decompress,
            unused_data: vm.ctx.empty_bytes.clone(),
            input_buffer: Vec::new(),
            eof: false,
            needs_input: true,
        }
    }

    pub fn eof(&self) -> bool {
        self.eof
    }

    pub fn unused_data(&self) -> PyBytesRef {
        self.unused_data.clone()
    }

    pub fn needs_input(&self) -> bool {
        self.needs_input
    }

    pub fn decompress(
        &mut self,
        data: &[u8],
        max_length: Option<usize>,
        bufsize: usize,
        vm: &VirtualMachine,
    ) -> Result<Vec<u8>, DecompressError<D::Error>> {
        if self.eof {
            return Err(DecompressError::Eof(EofError));
        }

        let input_buffer = &mut self.input_buffer;
        let d = &mut self.decompress;

        let mut chunks = Chunker::chain(input_buffer, data);

        let prev_len = chunks.len();
        let (ret, stream_end) =
            match _decompress_chunks(&mut chunks, d, bufsize, max_length, flush_sync) {
                Ok((buf, stream_end)) => (Ok(buf), stream_end),
                Err(err) => (Err(err), false),
            };
        let consumed = prev_len - chunks.len();

        self.eof |= stream_end;

        if self.eof {
            self.needs_input = false;
            if !chunks.is_empty() {
                self.unused_data = vm.ctx.new_bytes(chunks.to_vec());
            }
        } else if chunks.is_empty() {
            input_buffer.clear();
            self.needs_input = true;
        } else {
            self.needs_input = false;
            if let Some(n_consumed_from_data) = consumed.checked_sub(input_buffer.len()) {
                input_buffer.clear();
                input_buffer.extend_from_slice(&data[n_consumed_from_data..]);
            } else {
                input_buffer.drain(..consumed);
                input_buffer.extend_from_slice(data);
            }
        }

        ret.map_err(DecompressError::Decompress)
    }
}

pub enum DecompressError<E> {
    Decompress(E),
    Eof(EofError),
}

impl<E> From<E> for DecompressError<E> {
    fn from(err: E) -> Self {
        Self::Decompress(err)
    }
}

pub struct EofError;

impl ToPyException for EofError {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_eof_error("End of stream already reached".to_owned())
    }
}
