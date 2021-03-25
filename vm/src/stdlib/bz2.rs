use std::fmt;
use std::io::Write;

use crate::builtins::pytype::PyTypeRef;
use crate::byteslike::PyBytesLike;
use crate::common::lock::PyMutex;
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, StaticType};
use crate::VirtualMachine;
use bzip2::write::BzEncoder;
use bzip2::{Decompress, Status};

// const BUFSIZ: i32 = 8192;

struct DecompressorState {
    decoder: Decompress,
    eof: bool,
    needs_input: bool,
    // input_buffer: Vec<u8>,
    // output_buffer: Vec<u8>,
}

#[pyclass(module = "_bz2", name = "BZ2Decompressor")]
struct BZ2Decompressor {
    state: PyMutex<DecompressorState>,
}

impl fmt::Debug for BZ2Decompressor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "_bz2.BZ2Compressor")
    }
}

impl PyValue for BZ2Decompressor {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl BZ2Decompressor {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        BZ2Decompressor {
            state: PyMutex::new(DecompressorState {
                decoder: Decompress::new(false),
                eof: false,
                needs_input: true,
                // input_buffer: Vec::new(),
                // output_buffer: Vec::new(),
            }),
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod]
    fn decompress(
        &self,
        data: PyBytesLike,
        // TODO: PyIntRef
        max_length: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let max_length = max_length.unwrap_or(-1);
        if max_length >= 0 {
            return Err(vm.new_not_implemented_error(
                "the max_value argument is not implemented yet".to_owned(),
            ));
        }
        // let max_length = if max_length < 0 || max_length >= BUFSIZ {
        //     BUFSIZ
        // } else {
        //     max_length
        // };

        let mut state = self.state.lock();
        let DecompressorState {
            decoder,
            eof,
            ..
            // needs_input,
            // input_buffer,
            // output_buffer,
        } = &mut *state;

        if *eof {
            return Err(vm.new_eof_error("End of stream already reached".to_owned()));
        }

        // data.with_ref(|data| input_buffer.extend(data));

        // If max_length is negative:
        // read the input X bytes at a time, compress it and append it to output.
        // Once you're out of input, setting needs_input to true and return the
        // output as bytes.
        //
        // TODO:
        // If max_length is non-negative:
        // Read the input X bytes at a time, compress it and append it to
        // the output. If output reaches `max_length` in size, return
        // it (up to max_length), and store the rest of the output
        // for later.

        // TODO: arbitrary choice, not the right way to do it.
        let mut buf = Vec::with_capacity(data.len() * 32);

        let before = decoder.total_in();
        let res = data.with_ref(|data| decoder.decompress_vec(data, &mut buf));
        let _written = (decoder.total_in() - before) as usize;

        let res = match res {
            Ok(x) => x,
            // TODO: error message
            _ => return Err(vm.new_os_error("Invalid data stream".to_owned()))
        };

        if res == Status::StreamEnd {
            *eof = true;
        }
        let out = vm.ctx.new_bytes(buf.to_vec());
        return Ok(out);
    }

    #[pyproperty]
    fn eof(&self, vm: &VirtualMachine) -> PyObjectRef {
        let state = self.state.lock();
        vm.ctx.new_bool(state.eof)
    }

    #[pyproperty]
    fn unused_data(&self, vm: &VirtualMachine) -> PyObjectRef {
        // Data found after the end of the compressed stream.
        // If this attribute is accessed before the end of the stream
        // has been reached, its value will be b''.
        vm.ctx.new_bytes(b"".to_vec())
        // alternatively, be more honest:
        // Err(vm.new_not_implemented_error(
        //     "unused_data isn't implemented yet".to_owned(),
        // ))
        //
        // TODO
        // let state = self.state.lock();
        // if state.eof {
        //     vm.ctx.new_bytes(state.input_buffer.to_vec())
        // else {
        //     vm.ctx.new_bytes(b"".to_vec())
        // }
    }

    #[pyproperty]
    fn needs_input(&self, vm: &VirtualMachine) -> PyObjectRef {
        // False if the decompress() method can provide more
        // decompressed data before requiring new uncompressed input.
        let state = self.state.lock();
        vm.ctx.new_bool(state.needs_input)
    }

    // TODO: mro()?
}

struct CompressorState {
    flushed: bool,
    encoder: Option<BzEncoder<Vec<u8>>>,
}

#[pyclass(module = "_bz2", name = "BZ2Compressor")]
struct BZ2Compressor {
    state: PyMutex<CompressorState>,
}

impl fmt::Debug for BZ2Compressor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "_bz2.BZ2Compressor")
    }
}

impl PyValue for BZ2Compressor {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

// TODO: return partial results from compress() instead of returning everything in flush()
#[pyimpl]
impl BZ2Compressor {
    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        compresslevel: OptionalArg<i32>, // TODO: PyIntRef
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        // TODO: seriously?
        // compresslevel.unwrap_or(bzip2::Compression::best().level().try_into().unwrap());
        let compresslevel = compresslevel.unwrap_or(9);
        let level = match compresslevel {
            valid_level @ 1..=9 => bzip2::Compression::new(valid_level as u32),
            _ => return Err(vm.new_value_error("compresslevel must be between 1 and 9".to_owned())),
        };

        BZ2Compressor {
            state: PyMutex::new(CompressorState {
                flushed: false,
                encoder: Some(BzEncoder::new(Vec::new(), level)),
            }),
        }
        .into_ref_with_type(vm, cls)
    }
    #[pymethod]
    fn compress(&self, data: PyBytesLike, vm: &VirtualMachine) -> PyResult {
        let mut state = self.state.lock();
        if state.flushed {
            return Err(vm.new_value_error("Compressor has been flushed".to_owned()));
        }

        // let CompressorState { flushed, encoder } = &mut *state;
        let CompressorState { encoder, .. } = &mut *state;

        // TODO: handle Err
        data.with_ref(|input_bytes| encoder.as_mut().unwrap().write_all(input_bytes).unwrap());
        Ok(vm.ctx.new_bytes(Vec::new()))
    }

    #[pymethod]
    fn flush(&self, vm: &VirtualMachine) -> PyResult {
        let mut state = self.state.lock();
        if state.flushed {
            return Err(vm.new_value_error("Repeated call to flush()".to_owned()));
        }

        // let CompressorState { flushed, encoder } = &mut *state;
        let CompressorState { encoder, .. } = &mut *state;

        // TODO: handle Err
        let out = encoder.take().unwrap().finish().unwrap();
        state.flushed = true;
        Ok(vm.ctx.new_bytes(out.to_vec()))
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "_bz2", {
        "BZ2Decompressor" => BZ2Decompressor::make_class(ctx),
        "BZ2Compressor" => BZ2Compressor::make_class(ctx),
    })
}
