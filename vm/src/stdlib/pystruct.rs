/*
 * Python struct module.
 *
 * Docs: https://docs.python.org/3/library/struct.html
 *
 * renamed to pystruct since struct is a rust keyword.
 *
 * Use this rust module to do byte packing:
 * https://docs.rs/byteorder/1.2.6/byteorder/
 */

use std::io::{Cursor, Read, Write};
use std::iter::Peekable;

use byteorder::{ReadBytesExt, WriteBytesExt};
use num_bigint::BigInt;
use num_traits::ToPrimitive;

use crate::function::PyFuncArgs;
use crate::obj::{objbool, objbytes, objfloat, objint, objstr, objtype};
use crate::pyobject::{PyObjectRef, PyResult};
use crate::VirtualMachine;

#[derive(Debug)]
struct FormatSpec {
    endianness: Endianness,
    codes: Vec<FormatCode>,
}

#[derive(Debug)]
enum Endianness {
    Native,
    Little,
    Big,
    Network,
}

#[derive(Debug)]
struct FormatCode {
    code: char,
}

fn parse_format_string(fmt: String) -> Result<FormatSpec, String> {
    let mut chars = fmt.chars().peekable();

    // First determine "<", ">","!" or "="
    let endianness = parse_endiannes(&mut chars);

    // Now, analyze struct string furter:
    let codes = parse_format_codes(&mut chars)?;

    Ok(FormatSpec { endianness, codes })
}

/// Parse endianness
/// See also: https://docs.python.org/3/library/struct.html?highlight=struct#byte-order-size-and-alignment
fn parse_endiannes<I>(chars: &mut Peekable<I>) -> Endianness
where
    I: Sized + Iterator<Item = char>,
{
    match chars.peek() {
        Some('@') => {
            chars.next().unwrap();
            Endianness::Native
        }
        Some('=') => {
            chars.next().unwrap();
            Endianness::Native
        }
        Some('<') => {
            chars.next().unwrap();
            Endianness::Little
        }
        Some('>') => {
            chars.next().unwrap();
            Endianness::Big
        }
        Some('!') => {
            chars.next().unwrap();
            Endianness::Network
        }
        _ => Endianness::Native,
    }
}

fn parse_format_codes<I>(chars: &mut Peekable<I>) -> Result<Vec<FormatCode>, String>
where
    I: Sized + Iterator<Item = char>,
{
    let mut codes = vec![];
    for c in chars {
        match c {
            'b' | 'B' | 'h' | 'H' | 'i' | 'I' | 'l' | 'L' | 'q' | 'Q' | 'f' | 'd' => {
                codes.push(FormatCode { code: c })
            }
            c => {
                return Err(format!("Illegal format code {:?}", c));
            }
        }
    }

    Ok(codes)
}

fn get_int(vm: &VirtualMachine, arg: &PyObjectRef) -> PyResult<BigInt> {
    objint::to_int(vm, arg, &BigInt::from(10))
}

fn pack_i8(vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()> {
    let v = get_int(vm, arg)?.to_i8().unwrap();
    data.write_i8(v).unwrap();
    Ok(())
}

fn pack_u8(vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()> {
    let v = get_int(vm, arg)?.to_u8().unwrap();
    data.write_u8(v).unwrap();
    Ok(())
}

fn pack_bool(vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()> {
    if objtype::isinstance(&arg, &vm.ctx.bool_type()) {
        let v = if objbool::get_value(arg) { 1 } else { 0 };
        data.write_u8(v).unwrap();
        Ok(())
    } else {
        Err(vm.new_type_error("Expected boolean".to_string()))
    }
}

fn pack_i16<Endianness>(
    vm: &VirtualMachine,
    arg: &PyObjectRef,
    data: &mut dyn Write,
) -> PyResult<()>
where
    Endianness: byteorder::ByteOrder,
{
    let v = get_int(vm, arg)?.to_i16().unwrap();
    data.write_i16::<Endianness>(v).unwrap();
    Ok(())
}

fn pack_u16<Endianness>(
    vm: &VirtualMachine,
    arg: &PyObjectRef,
    data: &mut dyn Write,
) -> PyResult<()>
where
    Endianness: byteorder::ByteOrder,
{
    let v = get_int(vm, arg)?.to_u16().unwrap();
    data.write_u16::<Endianness>(v).unwrap();
    Ok(())
}

fn pack_i32<Endianness>(
    vm: &VirtualMachine,
    arg: &PyObjectRef,
    data: &mut dyn Write,
) -> PyResult<()>
where
    Endianness: byteorder::ByteOrder,
{
    let v = get_int(vm, arg)?.to_i32().unwrap();
    data.write_i32::<Endianness>(v).unwrap();
    Ok(())
}

fn pack_u32<Endianness>(
    vm: &VirtualMachine,
    arg: &PyObjectRef,
    data: &mut dyn Write,
) -> PyResult<()>
where
    Endianness: byteorder::ByteOrder,
{
    let v = get_int(vm, arg)?.to_u32().unwrap();
    data.write_u32::<Endianness>(v).unwrap();
    Ok(())
}

fn pack_i64<Endianness>(
    vm: &VirtualMachine,
    arg: &PyObjectRef,
    data: &mut dyn Write,
) -> PyResult<()>
where
    Endianness: byteorder::ByteOrder,
{
    let v = get_int(vm, arg)?.to_i64().unwrap();
    data.write_i64::<Endianness>(v).unwrap();
    Ok(())
}

fn pack_u64<Endianness>(
    vm: &VirtualMachine,
    arg: &PyObjectRef,
    data: &mut dyn Write,
) -> PyResult<()>
where
    Endianness: byteorder::ByteOrder,
{
    let v = get_int(vm, arg)?.to_u64().unwrap();
    data.write_u64::<Endianness>(v).unwrap();
    Ok(())
}

fn pack_f32<Endianness>(
    vm: &VirtualMachine,
    arg: &PyObjectRef,
    data: &mut dyn Write,
) -> PyResult<()>
where
    Endianness: byteorder::ByteOrder,
{
    let v = get_float(vm, arg)? as f32;
    data.write_f32::<Endianness>(v).unwrap();
    Ok(())
}

fn pack_f64<Endianness>(
    vm: &VirtualMachine,
    arg: &PyObjectRef,
    data: &mut dyn Write,
) -> PyResult<()>
where
    Endianness: byteorder::ByteOrder,
{
    let v = get_float(vm, arg)?;
    data.write_f64::<Endianness>(v).unwrap();
    Ok(())
}

fn get_float(vm: &VirtualMachine, arg: &PyObjectRef) -> PyResult<f64> {
    if objtype::isinstance(&arg, &vm.ctx.float_type()) {
        Ok(objfloat::get_value(arg))
    } else {
        Err(vm.new_type_error("Expected float".to_string()))
    }
}

fn pack_item<Endianness>(
    vm: &VirtualMachine,
    code: &FormatCode,
    arg: &PyObjectRef,
    data: &mut dyn Write,
) -> PyResult<()>
where
    Endianness: byteorder::ByteOrder,
{
    match code.code {
        'b' => pack_i8(vm, arg, data)?,
        'B' => pack_u8(vm, arg, data)?,
        '?' => pack_bool(vm, arg, data)?,
        'h' => pack_i16::<Endianness>(vm, arg, data)?,
        'H' => pack_u16::<Endianness>(vm, arg, data)?,
        'i' | 'l' => pack_i32::<Endianness>(vm, arg, data)?,
        'I' | 'L' => pack_u32::<Endianness>(vm, arg, data)?,
        'q' => pack_i64::<Endianness>(vm, arg, data)?,
        'Q' => pack_u64::<Endianness>(vm, arg, data)?,
        'f' => pack_f32::<Endianness>(vm, arg, data)?,
        'd' => pack_f64::<Endianness>(vm, arg, data)?,
        c => {
            panic!("Unsupported format code {:?}", c);
        }
    }
    Ok(())
}

fn struct_pack(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.is_empty() {
        Err(vm.new_type_error(format!(
            "Expected at least 1 argument (got: {})",
            args.args.len()
        )))
    } else {
        let fmt_arg = args.args[0].clone();
        if objtype::isinstance(&fmt_arg, &vm.ctx.str_type()) {
            let fmt_str = objstr::get_value(&fmt_arg);

            let format_spec = parse_format_string(fmt_str).map_err(|e| vm.new_value_error(e))?;

            if format_spec.codes.len() + 1 == args.args.len() {
                // Create data vector:
                let mut data = Vec::<u8>::new();
                // Loop over all opcodes:
                for (code, arg) in format_spec.codes.iter().zip(args.args.iter().skip(1)) {
                    debug!("code: {:?}", code);
                    match format_spec.endianness {
                        Endianness::Little => {
                            pack_item::<byteorder::LittleEndian>(vm, code, arg, &mut data)?
                        }
                        Endianness::Big => {
                            pack_item::<byteorder::BigEndian>(vm, code, arg, &mut data)?
                        }
                        Endianness::Network => {
                            pack_item::<byteorder::NetworkEndian>(vm, code, arg, &mut data)?
                        }
                        Endianness::Native => {
                            pack_item::<byteorder::NativeEndian>(vm, code, arg, &mut data)?
                        }
                    }
                }

                Ok(vm.ctx.new_bytes(data))
            } else {
                Err(vm.new_type_error(format!(
                    "Expected {} arguments (got: {})",
                    format_spec.codes.len() + 1,
                    args.args.len()
                )))
            }
        } else {
            Err(vm.new_type_error("First argument must be of str type".to_string()))
        }
    }
}

fn unpack_i8(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
    match rdr.read_i8() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u8(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
    match rdr.read_u8() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_bool(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
    match rdr.read_u8() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_bool(v > 0)),
    }
}

fn unpack_i16<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_i16::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u16<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_u16::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_i32<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_i32::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u32<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_u32::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_i64<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_i64::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u64<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_u64::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_f32<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_f32::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_float(f64::from(v))),
    }
}

fn unpack_f64<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_f64::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_float(v)),
    }
}

fn struct_unpack(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (fmt, Some(vm.ctx.str_type())),
            (buffer, Some(vm.ctx.bytes_type()))
        ]
    );

    let fmt_str = objstr::get_value(&fmt);

    let format_spec = parse_format_string(fmt_str).map_err(|e| vm.new_value_error(e))?;
    let data = objbytes::get_value(buffer).to_vec();
    let mut rdr = Cursor::new(data);

    let mut items = vec![];
    for code in format_spec.codes {
        debug!("unpack code: {:?}", code);
        let item = match format_spec.endianness {
            Endianness::Little => unpack_code::<byteorder::LittleEndian>(vm, &code, &mut rdr)?,
            Endianness::Big => unpack_code::<byteorder::BigEndian>(vm, &code, &mut rdr)?,
            Endianness::Network => unpack_code::<byteorder::NetworkEndian>(vm, &code, &mut rdr)?,
            Endianness::Native => unpack_code::<byteorder::NativeEndian>(vm, &code, &mut rdr)?,
        };
        items.push(item);
    }

    Ok(vm.ctx.new_tuple(items))
}

fn unpack_code<Endianness>(vm: &VirtualMachine, code: &FormatCode, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match code.code {
        'b' => unpack_i8(vm, rdr),
        'B' => unpack_u8(vm, rdr),
        '?' => unpack_bool(vm, rdr),
        'h' => unpack_i16::<Endianness>(vm, rdr),
        'H' => unpack_u16::<Endianness>(vm, rdr),
        'i' | 'l' => unpack_i32::<Endianness>(vm, rdr),
        'I' | 'L' => unpack_u32::<Endianness>(vm, rdr),
        'q' => unpack_i64::<Endianness>(vm, rdr),
        'Q' => unpack_u64::<Endianness>(vm, rdr),
        'f' => unpack_f32::<Endianness>(vm, rdr),
        'd' => unpack_f64::<Endianness>(vm, rdr),
        c => {
            panic!("Unsupported format code {:?}", c);
        }
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let struct_error = ctx.new_class("struct.error", ctx.object());

    py_module!(vm, "struct", {
        "pack" => ctx.new_rustfunc(struct_pack),
        "unpack" => ctx.new_rustfunc(struct_unpack),
        "error" => struct_error,
    })
}
