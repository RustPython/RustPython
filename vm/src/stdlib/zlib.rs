use crate::byteslike::PyBytesLike;
use crate::common::cell::PyMutex;
use crate::exceptions::PyBaseExceptionRef;
use crate::function::OptionalArg;
use crate::obj::objbytes::{PyBytes, PyBytesRef};
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{BorrowValue, IntoPyRef, PyClassImpl, PyObjectRef, PyResult, PyValue};
use crate::types::create_type;
use crate::vm::VirtualMachine;

use adler32::RollingAdler32 as Adler32;
use crc32fast::Hasher as Crc32;
use crossbeam_utils::atomic::AtomicCell;
use flate2::{
    write::ZlibEncoder, Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status,
};
use libz_sys as libz;

use std::io::Write;

// copied from zlibmodule.c (commit 530f506ac91338)
const MAX_WBITS: u8 = 15;
const DEF_BUF_SIZE: usize = 16 * 1024;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let zlib_error = create_type(
        "error",
        &ctx.types.type_type,
        &ctx.exceptions.exception_type,
    );

    py_module!(vm, "zlib", {
        "crc32" => ctx.new_function(zlib_crc32),
        "adler32" => ctx.new_function(zlib_adler32),
        "compress" => ctx.new_function(zlib_compress),
        "decompress" => ctx.new_function(zlib_decompress),
        "compressobj" => ctx.new_function(zlib_compressobj),
        "decompressobj" => ctx.new_function(zlib_decompressobj),
        "Compress" => PyCompress::make_class(ctx),
        "Decompress" => PyDecompress::make_class(ctx),
        "error" => zlib_error,
        "Z_DEFAULT_COMPRESSION" => ctx.new_int(libz::Z_DEFAULT_COMPRESSION),
        "Z_NO_COMPRESSION" => ctx.new_int(libz::Z_NO_COMPRESSION),
        "Z_BEST_SPEED" => ctx.new_int(libz::Z_BEST_SPEED),
        "Z_BEST_COMPRESSION" => ctx.new_int(libz::Z_BEST_COMPRESSION),
        "DEFLATED" => ctx.new_int(libz::Z_DEFLATED),
        "DEF_BUF_SIZE" => ctx.new_int(DEF_BUF_SIZE),
        "MAX_WBITS" => ctx.new_int(MAX_WBITS),
    })
}

/// Compute an Adler-32 checksum of data.
fn zlib_adler32(data: PyBytesRef, begin_state: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult {
    let data = data.borrow_value();

    let begin_state = begin_state.unwrap_or(1);

    let mut hasher = Adler32::from_value(begin_state as u32);
    hasher.update_buffer(data);

    let checksum: u32 = hasher.hash();

    Ok(vm.ctx.new_int(checksum))
}

/// Compute a CRC-32 checksum of data.
fn zlib_crc32(data: PyBytesRef, begin_state: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult {
    let data = data.borrow_value();

    let begin_state = begin_state.unwrap_or(0);

    let mut hasher = Crc32::new_with_initial(begin_state as u32);
    hasher.update(data);

    let checksum: u32 = hasher.finalize();

    Ok(vm.ctx.new_int(checksum))
}

/// Returns a bytes object containing compressed data.
fn zlib_compress(data: PyBytesLike, level: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult {
    let level = level.unwrap_or(libz::Z_DEFAULT_COMPRESSION);

    let compression = match level {
        valid_level @ libz::Z_NO_COMPRESSION..=libz::Z_BEST_COMPRESSION => {
            Compression::new(valid_level as u32)
        }
        libz::Z_DEFAULT_COMPRESSION => Compression::default(),
        _ => return Err(zlib_error("Bad compression level", vm)),
    };

    let mut encoder = ZlibEncoder::new(Vec::new(), compression);
    data.with_ref(|input_bytes| encoder.write_all(input_bytes).unwrap());
    let encoded_bytes = encoder.finish().unwrap();

    Ok(vm.ctx.new_bytes(encoded_bytes))
}

// TODO: validate wbits value here
fn header_from_wbits(wbits: OptionalArg<i8>) -> (bool, u8) {
    let wbits = wbits.unwrap_or(MAX_WBITS as i8);
    (wbits > 0, wbits.abs() as u8)
}

fn decompress(
    data: &[u8],
    d: &mut Decompress,
    bufsize: usize,
    max_length: Option<usize>,
    vm: &VirtualMachine,
) -> PyResult<(Vec<u8>, bool)> {
    if data.is_empty() {
        return Ok((Vec::new(), true));
    }
    let orig_in = d.total_in();
    let mut buf = Vec::new();

    for mut chunk in data.chunks(CHUNKSIZE) {
        // if this is the final chunk, finish it
        let flush = if d.total_in() - orig_in == (data.len() - chunk.len()) as u64 {
            FlushDecompress::Finish
        } else {
            FlushDecompress::None
        };
        loop {
            let additional = if let Some(max_length) = max_length {
                std::cmp::min(bufsize, max_length - buf.capacity())
            } else {
                bufsize
            };

            buf.reserve_exact(additional);
            let prev_in = d.total_in();
            match d.decompress_vec(chunk, &mut buf, flush) {
                // we've reached the end of the stream, we're done
                Ok(Status::StreamEnd) => {
                    buf.shrink_to_fit();
                    return Ok((buf, true));
                }
                // we have hit the maximum length that we can decompress, so stop
                Ok(_) if max_length.map_or(false, |max_length| buf.len() == max_length) => {
                    return Ok((buf, false));
                }
                Ok(_) => {
                    chunk = &chunk[(d.total_in() - prev_in) as usize..];

                    if !chunk.is_empty() {
                        // there is more input to process
                        continue;
                    } else if flush == FlushDecompress::Finish {
                        if buf.len() == buf.capacity() {
                            // we've run out of space, loop again and allocate more room
                            continue;
                        } else {
                            // we need more input to continue
                            buf.shrink_to_fit();
                            return Ok((buf, false));
                        }
                    } else {
                        // progress onto next chunk
                        break;
                    }
                }
                Err(_) => return Err(zlib_error("invalid input data", vm)),
            }
        }
    }
    unreachable!("Didn't reach end of stream or capacity limit")
}

/// Returns a bytes object containing the uncompressed data.
fn zlib_decompress(
    data: PyBytesLike,
    wbits: OptionalArg<i8>,
    bufsize: OptionalArg<usize>,
    vm: &VirtualMachine,
) -> PyResult<Vec<u8>> {
    data.with_ref(|data| {
        let (header, wbits) = header_from_wbits(wbits);
        let bufsize = bufsize.unwrap_or(DEF_BUF_SIZE);

        let mut d = Decompress::new_with_window_bits(header, wbits);
        decompress(data, &mut d, bufsize, None, vm).and_then(|(buf, stream_end)| {
            if stream_end {
                Ok(buf)
            } else {
                Err(zlib_error("incomplete or truncated stream", vm))
            }
        })
    })
}

fn zlib_decompressobj(
    wbits: OptionalArg<i8>,
    zdict: OptionalArg<PyBytesLike>,
    vm: &VirtualMachine,
) -> PyDecompress {
    let (header, wbits) = header_from_wbits(wbits);
    let mut decompress = Decompress::new_with_window_bits(header, wbits);
    if let OptionalArg::Present(dict) = zdict {
        dict.with_ref(|d| decompress.set_dictionary(d).unwrap());
    }
    PyDecompress {
        decompress: PyMutex::new(decompress),
        eof: AtomicCell::new(false),
        unused_data: PyMutex::new(PyBytes::from(vec![]).into_ref(vm)),
        unconsumed_tail: PyMutex::new(PyBytes::from(vec![]).into_ref(vm)),
    }
}
#[pyclass(name = "Decompress")]
#[derive(Debug)]
struct PyDecompress {
    decompress: PyMutex<Decompress>,
    eof: AtomicCell<bool>,
    unused_data: PyMutex<PyBytesRef>,
    unconsumed_tail: PyMutex<PyBytesRef>,
}
impl PyValue for PyDecompress {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("zlib", "Decompress")
    }
}
#[pyimpl]
impl PyDecompress {
    #[pyproperty]
    fn eof(&self) -> bool {
        self.eof.load()
    }
    #[pyproperty]
    fn unused_data(&self) -> PyBytesRef {
        self.unused_data.lock().clone()
    }
    #[pyproperty]
    fn unconsumed_tail(&self) -> PyBytesRef {
        self.unconsumed_tail.lock().clone()
    }

    fn save_unused_input(
        &self,
        d: &mut Decompress,
        data: &[u8],
        stream_end: bool,
        orig_in: u64,
        vm: &VirtualMachine,
    ) {
        let leftover = &data[(d.total_in() - orig_in) as usize..];

        if stream_end && !leftover.is_empty() {
            let mut unused_data = self.unused_data.lock();
            let unused: Vec<_> = unused_data
                .borrow_value()
                .iter()
                .chain(leftover)
                .copied()
                .collect();
            *unused_data = unused.into_pyref(vm);
        }
    }

    #[pymethod]
    fn decompress(&self, args: DecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let max_length = if args.max_length == 0 {
            None
        } else {
            Some(args.max_length)
        };
        let data = args.data.borrow_value();

        let mut d = self.decompress.lock();
        let orig_in = d.total_in();

        let (ret, stream_end) = match decompress(data, &mut d, DEF_BUF_SIZE, max_length, vm) {
            Ok((buf, true)) => {
                self.eof.store(true);
                (Ok(buf), true)
            }
            Ok((buf, false)) => (Ok(buf), false),
            Err(err) => (Err(err), false),
        };
        self.save_unused_input(&mut d, data, stream_end, orig_in, vm);

        let leftover = if !stream_end {
            &data[(d.total_in() - orig_in) as usize..]
        } else {
            b""
        };
        let mut unconsumed_tail = self.unconsumed_tail.lock();
        if !leftover.is_empty() || unconsumed_tail.len() > 0 {
            *unconsumed_tail = PyBytes::from(leftover.to_owned()).into_ref(vm);
        }

        ret
    }

    #[pymethod]
    fn flush(&self, length: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let length = match length {
            OptionalArg::Present(l) => {
                if l <= 0 {
                    return Err(vm.new_value_error("length must be greater than zero".to_owned()));
                } else {
                    l as usize
                }
            }
            OptionalArg::Missing => DEF_BUF_SIZE,
        };

        let mut data = self.unconsumed_tail.lock();
        let mut d = self.decompress.lock();

        let orig_in = d.total_in();

        let (ret, stream_end) = match decompress(&data, &mut d, length, None, vm) {
            Ok((buf, stream_end)) => (Ok(buf), stream_end),
            Err(err) => (Err(err), false),
        };
        self.save_unused_input(&mut d, &data, stream_end, orig_in, vm);

        *data = PyBytes::from(Vec::new()).into_ref(vm);

        // TODO: drop the inner decompressor, somehow
        // if stream_end {
        //
        // }
        ret
    }
}

#[derive(FromArgs)]
struct DecompressArgs {
    #[pyarg(positional_only)]
    data: PyBytesRef,
    #[pyarg(positional_or_keyword, default = "0")]
    max_length: usize,
}

fn zlib_compressobj(
    level: OptionalArg<i32>,
    // only DEFLATED is valid right now, it's w/e
    _method: OptionalArg<i32>,
    wbits: OptionalArg<i8>,
    vm: &VirtualMachine,
) -> PyResult<PyCompress> {
    let (header, wbits) = header_from_wbits(wbits);
    let level = level.unwrap_or(-1);

    let level = match level {
        -1 => libz::Z_DEFAULT_COMPRESSION as u32,
        n @ 0..=9 => n as u32,
        _ => return Err(vm.new_value_error("invalid initialization option".to_owned())),
    };
    let compress = Compress::new_with_window_bits(Compression::new(level), header, wbits);
    Ok(PyCompress {
        inner: PyMutex::new(CompressInner {
            compress,
            unconsumed: Vec::new(),
        }),
    })
}

#[derive(Debug)]
struct CompressInner {
    compress: Compress,
    unconsumed: Vec<u8>,
}

#[pyclass]
#[derive(Debug)]
struct PyCompress {
    inner: PyMutex<CompressInner>,
}

impl PyValue for PyCompress {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("zlib", "Compress")
    }
}

#[pyimpl]
impl PyCompress {
    #[pymethod]
    fn compress(&self, data: PyBytesLike, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut inner = self.inner.lock();
        data.with_ref(|b| inner.compress(b, vm))
    }

    #[pymethod]
    fn flush(&self, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        self.inner.lock().flush(vm)
    }

    // TODO: This is optional feature of Compress
    // #[pymethod]
    // #[pymethod(magic)]
    // #[pymethod(name = "__deepcopy__")]
    // fn copy(&self) -> Self {
    //     todo!("<flate2::Compress as Clone>")
    // }
}

const CHUNKSIZE: usize = libc::c_uint::MAX as usize;

impl CompressInner {
    fn save_unconsumed_input(&mut self, data: &[u8], orig_in: u64) {
        let leftover = &data[(self.compress.total_in() - orig_in) as usize..];
        self.unconsumed.extend_from_slice(leftover);
    }

    fn compress(&mut self, data: &[u8], vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let orig_in = self.compress.total_in();
        let unconsumed = std::mem::take(&mut self.unconsumed);
        let mut buf = Vec::new();

        'outer: for chunk in unconsumed.chunks(CHUNKSIZE).chain(data.chunks(CHUNKSIZE)) {
            loop {
                buf.reserve(DEF_BUF_SIZE);
                match self
                    .compress
                    .compress_vec(chunk, &mut buf, FlushCompress::None)
                {
                    Ok(_) if buf.len() == buf.capacity() => continue,
                    Ok(Status::StreamEnd) => break 'outer,
                    Ok(_) => break,
                    Err(_) => {
                        self.save_unconsumed_input(data, orig_in);
                        return Err(zlib_error("error while compressing", vm));
                    }
                }
            }
        }
        self.save_unconsumed_input(data, orig_in);

        buf.shrink_to_fit();
        Ok(buf)
    }

    // TODO: flush mode (FlushDecompress) parameter
    fn flush(&mut self, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let data = std::mem::take(&mut self.unconsumed);
        let mut data_it = data.chunks(CHUNKSIZE);
        let mut buf = Vec::new();

        loop {
            let chunk = data_it.next().unwrap_or(&[]);
            if buf.len() == buf.capacity() {
                buf.reserve(DEF_BUF_SIZE);
            }
            match self
                .compress
                .compress_vec(chunk, &mut buf, FlushCompress::Finish)
            {
                Ok(Status::StreamEnd) => break,
                Ok(_) => continue,
                Err(_) => return Err(zlib_error("error while compressing", vm)),
            }
        }

        buf.shrink_to_fit();
        Ok(buf)
    }
}

fn zlib_error(message: &str, vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_exception_msg(vm.class("zlib", "error"), message.to_owned())
}
