use crate::function::OptionalArg;
use crate::obj::objbytes::PyBytesRef;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;
use crc::{crc32, Hasher32};

fn hex_nibble(n: u8) -> u8 {
    match n {
        0..=9 => b'0' + n,
        10..=15 => b'a' + n,
        _ => unreachable!(),
    }
}

fn binascii_hexlify(data: PyBytesRef, vm: &VirtualMachine) -> PyResult {
    let bytes = data.get_value();
    let mut hex = Vec::<u8>::with_capacity(bytes.len() * 2);
    for b in bytes.iter() {
        hex.push(hex_nibble(b >> 4));
        hex.push(hex_nibble(b & 0xf));
    }

    Ok(vm.ctx.new_bytes(hex))
}

fn unhex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn binascii_unhexlify(hexstr: PyBytesRef, vm: &VirtualMachine) -> PyResult {
    // TODO: allow 'str' hexstrings as well
    let hex_bytes = hexstr.get_value();
    if hex_bytes.len() % 2 != 0 {
        return Err(vm.new_value_error("Odd-length string".to_string()));
    }

    let mut unhex = Vec::<u8>::with_capacity(hex_bytes.len() / 2);
    for i in (0..hex_bytes.len()).step_by(2) {
        let n1 = unhex_nibble(hex_bytes[i]);
        let n2 = unhex_nibble(hex_bytes[i + 1]);
        if let (Some(n1), Some(n2)) = (n1, n2) {
            unhex.push(n1 << 4 | n2);
        } else {
            return Err(vm.new_value_error("Non-hexadecimal digit found".to_string()));
        }
    }

    Ok(vm.ctx.new_bytes(unhex))
}

fn binascii_crc32(data: PyBytesRef, value: OptionalArg<u32>, vm: &VirtualMachine) -> PyResult {
    let bytes = data.get_value();
    let crc = value.unwrap_or(0u32);

    let mut digest = crc32::Digest::new_with_initial(crc32::IEEE, crc);
    digest.write(&bytes);

    Ok(vm.ctx.new_int(digest.sum32()))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "binascii", {
        "hexlify" => ctx.new_rustfunc(binascii_hexlify),
        "b2a_hex" => ctx.new_rustfunc(binascii_hexlify),
        "unhexlify" => ctx.new_rustfunc(binascii_unhexlify),
        "a2b_hex" => ctx.new_rustfunc(binascii_unhexlify),
        "crc32" => ctx.new_rustfunc(binascii_crc32),
    })
}
