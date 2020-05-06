use crate::exceptions::PyBaseExceptionRef;
use crate::function::OptionalArg;
use crate::obj::objbyteinner::PyBytesLike;
use crate::obj::objbytes::{PyBytes, PyBytesRef};
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyResult, PyValue};
use crate::types::create_type;
use crate::vm::VirtualMachine;

use adler32::RollingAdler32 as Adler32;
use crc32fast::Hasher as Crc32;
use crossbeam_utils::atomic::AtomicCell;
use flate2::{write::ZlibEncoder, Compression, Decompress, FlushDecompress, Status};
use libz_sys as libz;

use std::io::Write;
use std::sync::Mutex;

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
        // "compressobj" => ctx.new_function(zlib_compressobj),
        "decompressobj" => ctx.new_function(zlib_decompressobj),
        "Decompress" => PyDecompress::make_class(ctx),
        "error" => zlib_error,
        "Z_DEFAULT_COMPRESSION" => ctx.new_int(libz::Z_DEFAULT_COMPRESSION),
        "Z_NO_COMPRESSION" => ctx.new_int(libz::Z_NO_COMPRESSION),
        "Z_BEST_SPEED" => ctx.new_int(libz::Z_BEST_SPEED),
        "Z_BEST_COMPRESSION" => ctx.new_int(libz::Z_BEST_COMPRESSION),
        "DEF_BUF_SIZE" => ctx.new_int(DEF_BUF_SIZE),
        "MAX_WBITS" => ctx.new_int(MAX_WBITS),
    })
}

/// Compute an Adler-32 checksum of data.
fn zlib_adler32(data: PyBytesRef, begin_state: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult {
    let data = data.get_value();

    let begin_state = begin_state.unwrap_or(1);

    let mut hasher = Adler32::from_value(begin_state as u32);
    hasher.update_buffer(data);

    let checksum: u32 = hasher.hash();

    Ok(vm.new_int(checksum))
}

/// Compute a CRC-32 checksum of data.
fn zlib_crc32(data: PyBytesRef, begin_state: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult {
    let data = data.get_value();

    let begin_state = begin_state.unwrap_or(0);

    let mut hasher = Crc32::new_with_initial(begin_state as u32);
    hasher.update(data);

    let checksum: u32 = hasher.finalize();

    Ok(vm.new_int(checksum))
}

/// Returns a bytes object containing compressed data.
fn zlib_compress(data: PyBytesRef, level: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult {
    let input_bytes = data.get_value();

    let level = level.unwrap_or(libz::Z_DEFAULT_COMPRESSION);

    let compression = match level {
        valid_level @ libz::Z_NO_COMPRESSION..=libz::Z_BEST_COMPRESSION => {
            Compression::new(valid_level as u32)
        }
        libz::Z_DEFAULT_COMPRESSION => Compression::default(),
        _ => return Err(zlib_error("Bad compression level", vm)),
    };

    let mut encoder = ZlibEncoder::new(Vec::new(), compression);
    encoder.write_all(input_bytes).unwrap();
    let encoded_bytes = encoder.finish().unwrap();

    Ok(vm.ctx.new_bytes(encoded_bytes))
}

// TODO: validate wbits value here
fn header_from_wbits(wbits: OptionalArg<i8>) -> (bool, u8) {
    let wbits = wbits.unwrap_or(MAX_WBITS as i8);
    (wbits > 0, wbits.abs() as u8)
}

/// Returns a bytes object containing the uncompressed data.
fn zlib_decompress(
    data: PyBytesRef,
    wbits: OptionalArg<i8>,
    bufsize: OptionalArg<usize>,
    vm: &VirtualMachine,
) -> PyResult<Vec<u8>> {
    let data = data.get_value();

    let (header, wbits) = header_from_wbits(wbits);
    let bufsize = bufsize.unwrap_or(DEF_BUF_SIZE);

    let mut d = Decompress::new_with_window_bits(header, wbits);
    let mut buf = Vec::new();

    // TODO: maybe deduplicate this with the Decompress.{decompress,flush}
    'outer: for chunk in data.chunks(libc::c_uint::max_value() as usize) {
        // if this is the final chunk, finish it
        let flush = if d.total_in() == (data.len() - chunk.len()) as u64 {
            FlushDecompress::Finish
        } else {
            FlushDecompress::None
        };
        loop {
            buf.reserve(bufsize);
            match d.decompress_vec(chunk, &mut buf, flush) {
                // we've run out of space, loop again and allocate more
                Ok(_) if buf.len() == buf.capacity() => {}
                // we've reached the end of the stream, we're done
                Ok(Status::StreamEnd) => {
                    break 'outer;
                }
                // we've reached the end of this chunk of the data, do the next one
                Ok(_) => break,
                Err(_) => return Err(zlib_error("invalid input data", vm)),
            }
        }
    }
    buf.shrink_to_fit();
    Ok(buf)
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
        decompress: Mutex::new(decompress),
        eof: AtomicCell::new(false),
        unused_data: Mutex::new(PyBytes::new(vec![]).into_ref(vm)),
        unconsumed_tail: Mutex::new(PyBytes::new(vec![]).into_ref(vm)),
    }
}
#[pyclass(name = "Decompress")]
#[derive(Debug)]
struct PyDecompress {
    decompress: Mutex<Decompress>,
    eof: AtomicCell<bool>,
    unused_data: Mutex<PyBytesRef>,
    unconsumed_tail: Mutex<PyBytesRef>,
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
        self.unused_data.lock().unwrap().clone()
    }
    #[pyproperty]
    fn unconsumed_tail(&self) -> PyBytesRef {
        self.unconsumed_tail.lock().unwrap().clone()
    }

    fn save_unconsumed_input(
        &self,
        d: &mut Decompress,
        data: &[u8],
        stream_end: bool,
        orig_in: u64,
        vm: &VirtualMachine,
    ) {
        let leftover = &data[(d.total_in() - orig_in) as usize..];

        if stream_end && !leftover.is_empty() {
            let mut unused_data = self.unused_data.lock().unwrap();
            let unused = unused_data
                .get_value()
                .iter()
                .chain(leftover)
                .copied()
                .collect();
            *unused_data = PyBytes::new(unused).into_ref(vm);
        }

        let mut unconsumed_tail = self.unconsumed_tail.lock().unwrap();
        if !leftover.is_empty() || unconsumed_tail.len() > 0 {
            *unconsumed_tail = PyBytes::new(leftover.to_owned()).into_ref(vm);
        }
    }

    #[pymethod]
    fn decompress(&self, args: DecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let limited = args.max_length == 0;
        let data = args.data.get_value();

        let mut d = self.decompress.lock().unwrap();

        let orig_in = d.total_in();
        let mut buf = Vec::new();
        let mut stream_end = false;

        'outer: for chunk in data.chunks(libc::c_uint::max_value() as usize) {
            // if this is the final chunk, finish it
            let flush = if d.total_in() - orig_in == (data.len() - chunk.len()) as u64 {
                FlushDecompress::Finish
            } else {
                FlushDecompress::None
            };
            loop {
                let additional = if limited {
                    std::cmp::min(DEF_BUF_SIZE, args.max_length - buf.capacity())
                } else {
                    DEF_BUF_SIZE
                };
                buf.reserve(additional);
                match d.decompress_vec(chunk, &mut buf, flush) {
                    // we've run out of space
                    Ok(_) if buf.len() == buf.capacity() => {
                        if limited && buf.len() == args.max_length {
                            // if we have a maximum length we can decompress and we've hit it, stop
                            break 'outer;
                        } else {
                            // otherwise, loop again and allocate more
                            continue;
                        }
                    }
                    // we've reached the end of the stream, we're done
                    Ok(Status::StreamEnd) => {
                        stream_end = true;
                        self.eof.store(true);
                        break 'outer;
                    }
                    // we've reached the end of this chunk of the data, do the next one
                    Ok(_) => break,
                    Err(_) => {
                        self.save_unconsumed_input(&mut d, data, stream_end, orig_in, vm);
                        return Err(zlib_error("invalid input data", vm));
                    }
                }
            }
        }
        buf.shrink_to_fit();
        self.save_unconsumed_input(&mut d, data, stream_end, orig_in, vm);
        Ok(buf)
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

        let data = self.unconsumed_tail.lock().unwrap();
        let mut d = self.decompress.lock().unwrap();

        let orig_in = d.total_in();
        let mut buf = Vec::new();
        let mut stream_end = false;

        'outer: for chunk in data.chunks(libc::c_uint::max_value() as usize) {
            // if this is the final chunk, finish it
            let flush = if d.total_in() - orig_in == (data.len() - chunk.len()) as u64 {
                FlushDecompress::Finish
            } else {
                FlushDecompress::None
            };
            loop {
                buf.reserve(length);
                match d.decompress_vec(chunk, &mut buf, flush) {
                    // we've run out of space, loop again and allocate more
                    Ok(_) if buf.len() == buf.capacity() => {}
                    // we've reached the end of the stream, we're done
                    Ok(Status::StreamEnd) => {
                        stream_end = true;
                        self.eof.store(true);
                        // self->is_initialised = 0;
                        break 'outer;
                    }
                    // we've reached the end of this chunk of the data, do the next one
                    Ok(_) => break,
                    Err(_) => {
                        self.save_unconsumed_input(&mut d, &data, stream_end, orig_in, vm);
                        return Err(zlib_error("invalid input data", vm));
                    }
                }
            }
        }
        buf.shrink_to_fit();
        self.save_unconsumed_input(&mut d, &data, stream_end, orig_in, vm);
        // TODO: drop the inner decompressor, somehow
        // if stream_end {
        //
        // }
        Ok(buf)
    }
}

#[derive(FromArgs)]
struct DecompressArgs {
    #[pyarg(positional_only)]
    data: PyBytesRef,
    #[pyarg(positional_or_keyword, default = "0")]
    max_length: usize,
}

fn zlib_error(message: &str, vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_exception_msg(vm.class("zlib", "error"), message.to_owned())
}
