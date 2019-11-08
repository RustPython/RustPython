use crate::function::OptionalArg;
use crate::obj::objbytes::PyBytesRef;
use crate::pyobject::{ItemProtocol, PyObjectRef, PyResult};
use crate::types::create_type;
use crate::vm::VirtualMachine;

use adler32::RollingAdler32 as Adler32;
use crc32fast::Hasher as Crc32;
use flate2::{write::ZlibEncoder, Compression, Decompress, FlushDecompress, Status};
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
        "crc32" => ctx.new_rustfunc(zlib_crc32),
        "adler32" => ctx.new_rustfunc(zlib_adler32),
        "compress" => ctx.new_rustfunc(zlib_compress),
        "decompress" => ctx.new_rustfunc(zlib_decompress),
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

/// Returns a bytes object containing the uncompressed data.
fn zlib_decompress(
    data: PyBytesRef,
    wbits: OptionalArg<u8>,
    bufsize: OptionalArg<usize>,
    vm: &VirtualMachine,
) -> PyResult {
    let encoded_bytes = data.get_value();

    let wbits = wbits.unwrap_or(MAX_WBITS);
    let bufsize = bufsize.unwrap_or(DEF_BUF_SIZE);

    let mut decompressor = Decompress::new_with_window_bits(true, wbits);
    let mut decoded_bytes = Vec::with_capacity(bufsize);

    match decompressor.decompress_vec(&encoded_bytes, &mut decoded_bytes, FlushDecompress::Finish) {
        Ok(Status::BufError) => Err(zlib_error("inconsistent or truncated state", vm)),
        Err(_) => Err(zlib_error("invalid input data", vm)),
        _ => Ok(vm.ctx.new_bytes(decoded_bytes)),
    }
}

fn zlib_error(message: &str, vm: &VirtualMachine) -> PyObjectRef {
    let module = vm
        .get_attribute(vm.sys_module.clone(), "modules")
        .unwrap()
        .get_item("zlib", vm)
        .unwrap();

    let zlib_error = vm.get_attribute(module, "error").unwrap();
    let zlib_error = zlib_error.downcast().unwrap();

    vm.new_exception(zlib_error, message.to_string())
}
