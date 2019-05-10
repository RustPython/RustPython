use crate::function::PyFuncArgs;
use crate::obj::objbytes;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

fn hex_nibble(n: u8) -> u8 {
    match n {
        0..=9 => b'0' + n,
        10..=15 => b'a' + n,
        _ => unreachable!(),
    }
}

fn binascii_hexlify(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(data, Some(vm.ctx.bytes_type()))]);

    let bytes = objbytes::get_value(data);
    let mut hex = Vec::<u8>::with_capacity(bytes.len() * 2);
    for b in bytes.iter() {
        hex.push(hex_nibble(b >> 4));
        hex.push(hex_nibble(b & 0xf));
    }

    Ok(vm.ctx.new_bytes(hex))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "binascii", {
        "hexlify" => ctx.new_rustfunc(binascii_hexlify),
    })
}
