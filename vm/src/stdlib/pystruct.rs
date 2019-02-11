/*
 * Python struct module.
 *
 * renamed to pystruct since struct is a rust keyword.
 *
 * Use this rust module to do byte packing:
 * https://docs.rs/byteorder/1.2.6/byteorder/
 */

extern crate byteorder;
use self::byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::super::obj::{objbool, objbytes, objfloat, objint, objstr, objtype};
use super::super::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use super::super::VirtualMachine;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::io::{Cursor, Read, Write};

#[derive(Debug)]
struct FormatCode {
    code: char,
    repeat: i32,
}

fn parse_format_string(fmt: String) -> Vec<FormatCode> {
    // First determine "<", ">","!" or "="
    // TODO

    // Now, analyze struct string furter:
    let mut codes = vec![];
    for c in fmt.chars() {
        match c {
            'b' | 'B' | 'h' | 'H' | 'i' | 'I' | 'q' | 'Q' | 'f' | 'd' => {
                codes.push(FormatCode { code: c, repeat: 1 })
            }
            c => {
                panic!("Illegal format code {:?}", c);
            }
        }
    }
    codes
}

fn get_int(vm: &mut VirtualMachine, arg: &PyObjectRef) -> Result<BigInt, PyObjectRef> {
    objint::to_int(vm, arg, 10)
}

fn pack_i8(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    let v = get_int(vm, arg)?.to_i8().unwrap();
    data.write_i8(v).unwrap();
    Ok(())
}

fn pack_u8(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    let v = get_int(vm, arg)?.to_u8().unwrap();
    data.write_u8(v).unwrap();
    Ok(())
}

fn pack_bool(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    if objtype::isinstance(&arg, &vm.ctx.bool_type()) {
        let v = if objbool::get_value(arg) { 1 } else { 0 };
        data.write_u8(v).unwrap();
        Ok(())
    } else {
        Err(vm.new_type_error("Expected boolean".to_string()))
    }
}

fn pack_i16(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    let v = get_int(vm, arg)?.to_i16().unwrap();
    data.write_i16::<LittleEndian>(v).unwrap();
    Ok(())
}

fn pack_u16(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    let v = get_int(vm, arg)?.to_u16().unwrap();
    data.write_u16::<LittleEndian>(v).unwrap();
    Ok(())
}

fn pack_i32(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    let v = get_int(vm, arg)?.to_i32().unwrap();
    data.write_i32::<LittleEndian>(v).unwrap();
    Ok(())
}

fn pack_u32(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    let v = get_int(vm, arg)?.to_u32().unwrap();
    data.write_u32::<LittleEndian>(v).unwrap();
    Ok(())
}

fn pack_i64(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    let v = get_int(vm, arg)?.to_i64().unwrap();
    data.write_i64::<LittleEndian>(v).unwrap();
    Ok(())
}

fn pack_u64(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    let v = get_int(vm, arg)?.to_u64().unwrap();
    data.write_u64::<LittleEndian>(v).unwrap();
    Ok(())
}

fn pack_f32(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    if objtype::isinstance(&arg, &vm.ctx.float_type()) {
        let v = objfloat::get_value(arg) as f32;
        data.write_f32::<LittleEndian>(v).unwrap();
        Ok(())
    } else {
        Err(vm.new_type_error("Expected float".to_string()))
    }
}

fn pack_f64(
    vm: &mut VirtualMachine,
    arg: &PyObjectRef,
    data: &mut Write,
) -> Result<(), PyObjectRef> {
    if objtype::isinstance(&arg, &vm.ctx.float_type()) {
        let v = objfloat::get_value(arg) as f64;
        data.write_f64::<LittleEndian>(v).unwrap();
        Ok(())
    } else {
        Err(vm.new_type_error("Expected float".to_string()))
    }
}

fn struct_pack(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.is_empty() {
        Err(vm.new_type_error(format!(
            "Expected at least 1 argument (got: {})",
            args.args.len()
        )))
    } else {
        let fmt_arg = args.args[0].clone();
        if objtype::isinstance(&fmt_arg, &vm.ctx.str_type()) {
            let fmt_str = objstr::get_value(&fmt_arg);

            let codes = parse_format_string(fmt_str);

            if codes.len() + 1 == args.args.len() {
                // Create data vector:
                let mut data = Vec::<u8>::new();
                // Loop over all opcodes:
                for (code, arg) in codes.iter().zip(args.args.iter().skip(1)) {
                    debug!("code: {:?}", code);
                    match code.code {
                        'b' => pack_i8(vm, arg, &mut data)?,
                        'B' => pack_u8(vm, arg, &mut data)?,
                        '?' => pack_bool(vm, arg, &mut data)?,
                        'h' => pack_i16(vm, arg, &mut data)?,
                        'H' => pack_u16(vm, arg, &mut data)?,
                        'i' => pack_i32(vm, arg, &mut data)?,
                        'I' => pack_u32(vm, arg, &mut data)?,
                        'l' => pack_i32(vm, arg, &mut data)?,
                        'L' => pack_u32(vm, arg, &mut data)?,
                        'q' => pack_i64(vm, arg, &mut data)?,
                        'Q' => pack_u64(vm, arg, &mut data)?,
                        'f' => pack_f32(vm, arg, &mut data)?,
                        'd' => pack_f64(vm, arg, &mut data)?,
                        c => {
                            panic!("Unsupported format code {:?}", c);
                        }
                    }
                }

                Ok(vm.ctx.new_bytes(data))
            } else {
                Err(vm.new_type_error(format!(
                    "Expected {} arguments (got: {})",
                    codes.len() + 1,
                    args.args.len()
                )))
            }
        } else {
            Err(vm.new_type_error("First argument must be of str type".to_string()))
        }
    }
}

fn unpack_i8(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_i8() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u8(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_u8() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_bool(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_u8() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_bool(v > 0)),
    }
}

fn unpack_i16(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_i16::<LittleEndian>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u16(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_u16::<LittleEndian>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_i32(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_i32::<LittleEndian>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u32(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_u32::<LittleEndian>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_i64(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_i64::<LittleEndian>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u64(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_u64::<LittleEndian>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_f32(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_f32::<LittleEndian>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_float(f64::from(v))),
    }
}

fn unpack_f64(vm: &mut VirtualMachine, rdr: &mut Read) -> PyResult {
    match rdr.read_f64::<LittleEndian>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_float(v)),
    }
}

fn struct_unpack(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (fmt, Some(vm.ctx.str_type())),
            (buffer, Some(vm.ctx.bytes_type()))
        ]
    );

    let fmt_str = objstr::get_value(&fmt);

    let codes = parse_format_string(fmt_str);
    let data = objbytes::get_value(buffer).to_vec();
    let mut rdr = Cursor::new(data);

    let mut items = vec![];
    for code in codes {
        debug!("unpack code: {:?}", code);
        match code.code {
            'b' => items.push(unpack_i8(vm, &mut rdr)?),
            'B' => items.push(unpack_u8(vm, &mut rdr)?),
            '?' => items.push(unpack_bool(vm, &mut rdr)?),
            'h' => items.push(unpack_i16(vm, &mut rdr)?),
            'H' => items.push(unpack_u16(vm, &mut rdr)?),
            'i' => items.push(unpack_i32(vm, &mut rdr)?),
            'I' => items.push(unpack_u32(vm, &mut rdr)?),
            'l' => items.push(unpack_i32(vm, &mut rdr)?),
            'L' => items.push(unpack_u32(vm, &mut rdr)?),
            'q' => items.push(unpack_i64(vm, &mut rdr)?),
            'Q' => items.push(unpack_u64(vm, &mut rdr)?),
            'f' => items.push(unpack_f32(vm, &mut rdr)?),
            'd' => items.push(unpack_f64(vm, &mut rdr)?),
            c => {
                panic!("Unsupported format code {:?}", c);
            }
        }
    }

    Ok(vm.ctx.new_tuple(items))
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"struct".to_string(), ctx.new_scope(None));

    ctx.set_attr(&py_mod, "pack", ctx.new_rustfunc(struct_pack));
    ctx.set_attr(&py_mod, "unpack", ctx.new_rustfunc(struct_unpack));

    py_mod
}
