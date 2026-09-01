// spell-checker:ignore chunker

//! internal shared module for compression libraries

use crate::vm::function::{ArgBytesLike, ArgSize, OptionalArg};
use crate::vm::{
    VirtualMachine,
    builtins::{PyBaseExceptionRef, PyBytesRef},
    convert::ToPyException,
};
use rustpython_common::compression::Chunker;

#[derive(FromArgs)]
pub(crate) struct DecompressArgs {
    #[pyarg(positional)]
    data: ArgBytesLike,
    #[pyarg(any, optional)]
    max_length: OptionalArg<ArgSize>,
}

impl DecompressArgs {
    pub(crate) fn data(&self) -> crate::common::borrow::BorrowedValue<'_, [u8]> {
        self.data.borrow_buf()
    }
    pub(crate) fn raw_max_length(&self) -> Option<isize> {
        self.max_length.into_option().map(|ArgSize { value }| value)
    }

    // negative is None
    pub(crate) fn max_length(&self) -> Option<usize> {
        self.max_length
            .into_option()
            .and_then(|ArgSize { value }| usize::try_from(value).ok())
    }
}

pub(crate) trait Decompressor {
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

pub(crate) trait DecompressStatus {
    fn is_stream_end(&self) -> bool;
}

pub(crate) trait DecompressFlushKind: Copy {
    const SYNC: Self;
}

impl DecompressFlushKind for () {
    const SYNC: Self = ();
}

pub(crate) const fn flush_sync<T: DecompressFlushKind>(_final_chunk: bool) -> T {
    T::SYNC
}

pub(crate) fn _decompress_chunks<D: Decompressor>(
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
            let additional = core::cmp::min(bufsize, max_length - buf.capacity());
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
                    }
                    // next chunk
                    continue 'outer;
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

#[derive(Debug)]
pub(crate) struct DecompressState<D> {
    decompress: D,
    unused_data: PyBytesRef,
    input_buffer: Vec<u8>,
    eof: bool,
    needs_input: bool,
}

impl<D: Decompressor> DecompressState<D> {
    pub(crate) fn new(decompress: D, vm: &VirtualMachine) -> Self {
        Self {
            decompress,
            unused_data: vm.ctx.empty_bytes.clone(),
            input_buffer: Vec::new(),
            eof: false,
            needs_input: true,
        }
    }

    pub(crate) const fn eof(&self) -> bool {
        self.eof
    }

    pub(crate) fn unused_data(&self) -> PyBytesRef {
        self.unused_data.clone()
    }

    pub(crate) const fn needs_input(&self) -> bool {
        self.needs_input
    }

    pub(crate) fn decompress(
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

pub(crate) enum DecompressError<E> {
    Decompress(E),
    Eof(EofError),
}

impl<E> From<E> for DecompressError<E> {
    fn from(err: E) -> Self {
        Self::Decompress(err)
    }
}

pub(crate) struct EofError;

impl ToPyException for EofError {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_eof_error("End of stream already reached")
    }
}
